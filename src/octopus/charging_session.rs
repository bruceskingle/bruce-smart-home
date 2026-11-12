//! The charging sessions of a smart charge point, e.g. a myenergi zappi, or vehicle, with the energy added and its cost,
//! including sessions at public chargers.

use std::time::Duration;

use bruce_graphql::types::{Date, DateTime};
use time_tz::Tz;
use bruce_graphql::{GraphQLCaller, GraphQLQueryBuilder, ProgressMonitor};

use crate::CacheConfig;
use crate::data_set::{CancelToken, MonthlyDataSet};
use super::decimal::Decimal;
use super::graphql::meter::charging_sessions;
use crate::components::stacked_bar_graph::{StackedBarPeriod, StackedBarPoint, StackedBarSeries};

/// The most sessions asked for in one request, if this many are returned the rest are asked for in another.
const PAGE_SIZE: i32 = 100;

/// A smart charge planned or carried out within a session.
#[derive(Clone, PartialEq, Debug)]
pub struct Dispatch {
    pub start: DateTime,
    pub end: DateTime,
    /// e.g. SMART or BOOST
    pub kind: String,
    pub energy_kwh: Option<Decimal>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ChargingSession {
    pub start: DateTime,
    pub end: DateTime,
    /// e.g. SMART, BOOST or PUBLIC
    pub kind: String,
    /// What the session was charging to, e.g. a percentage state of charge.
    pub target: Option<String>,
    /// The energy added, and its unit, e.g. KILOWATT_HOURS.
    pub energy: Option<(Decimal, String)>,
    /// The cost, in the minor units of its currency, e.g. pence, and the currency code, e.g. GBP.
    pub cost: Option<(Decimal, String)>,
    /// The change in the vehicle's state of charge, and its state of charge at the end, as percentages.
    pub state_of_charge_change: Option<Decimal>,
    pub state_of_charge_final: Option<Decimal>,
    pub dispatches: Vec<Dispatch>,
    /// The cause of each thing which stopped the session charging as planned, e.g. FULL_CHARGE.
    pub problems: Vec<Problem>,
    /// Where a public charging session was, e.g. the charger's name and address.
    pub location: Option<String>,
}

/// Something which stopped a session charging as planned.
#[derive(Clone, PartialEq, Debug)]
pub enum Problem {
    /// The cause, e.g. FULL_CHARGE or NO_SCHEDULED_CHARGE
    Error(String),
    /// The cause, and the state of charge reached and the one which was possible, as percentages.
    Truncation(String, Decimal, Decimal),
}

impl Problem {
    /// The CSS class to show the problem with: ok for a full EV, which is a good outcome, invalid for a charger error.
    pub fn class(&self) -> &'static str {
        match self {
            Problem::Error(cause) if cause == "FULL_CHARGE" => "ok",
            Problem::Error(cause) if cause == "FAILURE_CAUSE_ERROR" => "invalid",
            _ => "",
        }
    }

    /// The problem in plain words.
    pub fn description(&self) -> String {
        match self {
            Problem::Error(cause) => match cause.as_str() {
                "FULL_CHARGE" => String::from("EV full"),
                "NO_SCHEDULED_CHARGE" => String::from("No charge scheduled"),
                "NO_CHARGING" => String::from("Did not charge"),
                "FAILURE_CAUSE_ERROR" => String::from("Charger error"),
                other => readable(other),
            },
            Problem::Truncation(cause, reached, possible) => format!("Cut short ({}), reached {reached}% of {possible}%", readable(cause)),
        }
    }
}

/// An energy unit as its symbol, e.g. kWh.
pub fn energy_unit(name: &str) -> String {
    match name {
        "KILOWATT_HOUR" => String::from("kWh"),
        "WATT_HOUR" => String::from("Wh"),
        "MEGAWATT_HOUR" => String::from("MWh"),
        other => readable(other),
    }
}

/// An enum value such as SOME_CAUSE as words, "Some cause".
pub fn readable(name: &str) -> String {
    let words = name.replace('_', " ").to_lowercase();
    let mut chars = words.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// An enum from the server as its name, it displays as a JSON string.
fn enum_name(value: impl ToString) -> String {
    value.to_string().trim_matches('"').to_string()
}

impl ChargingSession {
    fn from(node: charging_sessions::DeviceChargingSession) -> Self {
        match node {
            charging_sessions::DeviceChargingSession::SmartFlexChargingSession(session) => ChargingSession {
                start: session.device_charging_session_.start_,
                end: session.device_charging_session_.end_,
                kind: enum_name(&session.type_),
                target: Some(enum_name(&session.target_type_)),
                energy: session.energy_added_
                    .and_then(|energy| energy.value_.map(|value| (value, enum_name(&energy.unit_)))),
                cost: session.cost_.map(|cost| (cost.amount_, cost.currency_)),
                state_of_charge_change: session.state_of_charge_change_,
                state_of_charge_final: session.state_of_charge_final_,
                dispatches: session.dispatches_.into_iter()
                    .map(|dispatch| Dispatch {
                        start: dispatch.start_,
                        end: dispatch.end_,
                        kind: enum_name(&dispatch.type_),
                        energy_kwh: dispatch.energy_added_kwh_,
                    })
                    .collect(),
                problems: session.problems_.into_iter()
                    .map(|problem| match problem {
                        charging_sessions::SmartFlexChargingProblem::SmartFlexChargingError(error) =>
                            Problem::Error(enum_name(&error.cause_)),
                        charging_sessions::SmartFlexChargingProblem::SmartFlexChargingTruncation(truncation) => Problem::Truncation(
                            enum_name(&truncation.truncation_cause_),
                            truncation.achievable_state_of_charge_,
                            truncation.original_achievable_state_of_charge_,
                        ),
                    })
                    .collect(),
                location: None,
            },
            // A session at a public charger, e.g. on Electroverse.
            charging_sessions::DeviceChargingSession::PublicChargingSession(session) => ChargingSession {
                start: session.device_charging_session_.start_,
                end: session.device_charging_session_.end_,
                kind: String::from("PUBLIC"),
                target: None,
                energy: session.energy_added_
                    .and_then(|energy| energy.value_.map(|value| (value, enum_name(&energy.unit_)))),
                cost: session.cost_.map(|cost| (cost.amount_, cost.currency_)),
                state_of_charge_change: None,
                state_of_charge_final: None,
                dispatches: Vec::new(),
                problems: Vec::new(),
                location: Some(session.location_),
            },
        }
    }
}

/// The sessions in a response, as the server returns them.
fn response_sessions(response: &charging_sessions::Response) -> Vec<&charging_sessions::DeviceChargingSession> {
    response.devices_.iter()
        .filter_map(|device| match device {
            charging_sessions::SmartFlexDeviceInterface::SmartFlexChargePoint(charge_point) => charge_point.charging_sessions_.as_ref(),
            charging_sessions::SmartFlexDeviceInterface::SmartFlexVehicle(vehicle) => vehicle.charging_sessions_.as_ref(),
            _ => None,
        })
        .flat_map(|connection| connection.edges_.iter().filter_map(|edge| edge.node_.as_ref()))
        .collect()
}

/// The start and end of a session as the server returns it.
fn session_times(session: &charging_sessions::DeviceChargingSession) -> (DateTime, DateTime) {
    match session {
        charging_sessions::DeviceChargingSession::SmartFlexChargingSession(session) =>
            (session.device_charging_session_.start_, session.device_charging_session_.end_),
        charging_sessions::DeviceChargingSession::PublicChargingSession(session) =>
            (session.device_charging_session_.start_, session.device_charging_session_.end_),
    }
}

/// The charging sessions of the charge point or vehicle device_id which start from start up to end, in time order. They
/// are cached a month at a time, see MonthlyDataSet. The months which reload covers are fetched again whatever is
/// cached, in case it is wrong. If cancel is set the months not yet loaded are left out.
pub async fn fetch(account_number: &str, device_id: &str, start: &DateTime, end: &DateTime, reload: Option<(DateTime, DateTime)>,
    tz: &'static Tz, config: &CacheConfig, request_manager: &impl GraphQLCaller, progress: &(impl ProgressMonitor + ?Sized),
    cancel: &CancelToken) -> anyhow::Result<Vec<ChargingSession>>
{
    let hash_key = format!("{account_number}#{device_id}#ChargingSessions");
    let query = |after: DateTime, before: DateTime| charging_sessions::Query::builder()
        .with_account_number(account_number.to_string())
        .with_device_id(device_id.to_string())
        .with_after(after)
        .with_before(before)
        .with_first(PAGE_SIZE)
        .build();
    let month_end = |month: &Date| -> DateTime {
        let next = if month.month() == time::Month::December {
            Date::from_calendar_date(month.year() + 1, time::Month::January, 1)
        }
        else {
            Date::from_calendar_date(month.year(), month.month().next(), 1)
        };
        next.map(|next| next.at_midnight(tz)).unwrap_or(*end)
    };

    let mut sessions = Vec::new();
    let mut month = start.to_timezone(tz).to_date();
    let month_format = time::format_description::parse("[month repr:long] [year]").unwrap();
    while month.at_midnight(tz) < *end {
        if cancel.is_cancelled() {
            break;
        }
        progress.progress(format!("{} {}", if reload.is_some() { "Reloading" } else { "Loading" }, month.format(&month_format).unwrap_or_default()));
        let force = reload.is_some_and(|(reload_start, reload_end)| month.at_midnight(tz) < reload_end && reload_start < month_end(&month));
        let data_set = MonthlyDataSet::new(
            &hash_key,
            &month,
            Duration::from_hours(24),
            force,
            // From the last final session cached, or the start of the month.
            |month_start: &Date, last: Option<&charging_sessions::DeviceChargingSession>| {
                let after = last.map(|last| session_times(last).0).unwrap_or_else(|| month_start.at_midnight(tz));
                query(after, month_end(month_start))
            },
            // A full page may not be all of them. The server returns the earliest sessions, so the rest start after
            // the latest one returned, after is inclusive so a second later to not return the latest again.
            |month_start: &Date, response: &charging_sessions::Response| {
                let page = response_sessions(response);
                let latest = page.iter().map(|session| session_times(session).0).max()?;
                if (page.len() as i32) < PAGE_SIZE {
                    return None;
                }
                Some(DateTime::from_unix_timestamp(latest.unix_timestamp() + 1)
                    .and_then(|after| query(after, month_end(month_start))))
            },
            |response: charging_sessions::Response| {
                response.devices_.into_iter()
                    .filter_map(|device| match device {
                        charging_sessions::SmartFlexDeviceInterface::SmartFlexChargePoint(charge_point) => charge_point.charging_sessions_,
                        charging_sessions::SmartFlexDeviceInterface::SmartFlexVehicle(vehicle) => vehicle.charging_sessions_,
                        _ => None,
                    })
                    .flat_map(|connection| connection.edges_.into_iter().filter_map(|edge| edge.node_))
                    .collect::<Vec<_>>()
            },
            // Indexed by the start in the billing timezone, so that a session is cached in the month it starts in
            // there. A session may still change until a day after it ends.
            |session: &charging_sessions::DeviceChargingSession| {
                let (start, end) = session_times(session);
                (start.to_timezone(tz), *end < *DateTime::now(tz) - time::Duration::days(1))
            },
            config,
            request_manager,
            progress,
        ).await?;

        sessions.extend(data_set.data.into_values()
            .map(|(_, session)| ChargingSession::from(session))
            .filter(|session| session.start >= *start && session.start < *end));
        month = Date::from(*month_end(&month).to_timezone(tz).to_date());
    }
    sessions.sort_by_key(|session| session.start);
    Ok(sessions)
}

/// The categories sessions can be filtered by. A session is shown if its kind, Smart or Public, is, unless it is in
/// one of the other categories which is not.
pub const SMART: &str = "Smart";
pub const PUBLIC: &str = "Public";
pub const FULL: &str = "Full";
pub const NO_CHARGE: &str = "No Charge";
pub const ERROR: &str = "Error";
pub const CATEGORIES: [&str; 5] = [SMART, PUBLIC, FULL, NO_CHARGE, ERROR];

impl ChargingSession {
    pub fn is_public(&self) -> bool {
        self.kind == "PUBLIC"
    }

    /// Whether the session added no energy, e.g. when the EV was plugged in with no charge scheduled. Public sessions
    /// count as charging, they would not have been started otherwise.
    pub fn plugged_in_only(&self) -> bool {
        !self.is_public() && self.energy.as_ref().is_none_or(|(value, _)| !value.is_non_zero())
    }

    fn has_problem(&self, cause: &str) -> bool {
        self.problems.iter().any(|problem| matches!(problem, Problem::Error(error) if error == cause))
    }

    /// The energy added in kWh, 0 if it is not known.
    pub fn energy_kwh(&self) -> f64 {
        self.energy.as_ref().and_then(|(value, _)| value.to_string().parse().ok()).unwrap_or_default()
    }

    /// Whether the session is shown, given which categories are.
    pub fn shown(&self, visible: impl Fn(&str) -> bool) -> bool {
        visible(if self.is_public() { PUBLIC } else { SMART })
            && (visible(FULL) || !self.has_problem("FULL_CHARGE"))
            && (visible(NO_CHARGE) || !self.plugged_in_only())
            && (visible(ERROR) || !self.has_problem("FAILURE_CAUSE_ERROR"))
    }
}

/// The start and end of the period of the given length containing date, Raw is not a period so is a day.
fn period_bounds(date: Date, period: StackedBarPeriod, tz: &'static Tz) -> (DateTime, DateTime) {
    let (start, end) = match period {
        StackedBarPeriod::Week => {
            let monday = *date - time::Duration::days(date.weekday().number_days_from_monday() as i64);
            (monday, monday + time::Duration::days(7))
        },
        StackedBarPeriod::Month => {
            let first = date.replace_day(1).unwrap_or(*date);
            let next = if first.month() == time::Month::December {
                first.replace_year(first.year() + 1).and_then(|date| date.replace_month(time::Month::January))
            }
            else {
                first.replace_month(first.month().next())
            }.unwrap_or(first);
            (first, next)
        },
        _ => (*date, *date + time::Duration::days(1)),
    };
    (Date::from(start).at_midnight(tz), Date::from(end).at_midnight(tz))
}

/// The colours of the sessions in the graph, in turn.
const SESSION_COLORS: [&str; 8] = ["#06b6d4", "#f2c94c", "#bb6bd9", "#46ee43", "#f6671f", "#4c8ef2", "#ef5da8", "#a3e635"];

/// A graph of the energy added by sessions from start to end, with a series for each session, in turn in each of
/// SESSION_COLORS, so that all the parts of a session are the same colour. For Raw each session is a bar, otherwise
/// there is a bar for each period, including ones with no sessions, with a part for each session in it. A session
/// which crosses the end of a period is split, its energy shared by the time in each, as when in the session the
/// energy was added is not known.
pub fn graph_points(sessions: &[&ChargingSession], start: &DateTime, end: &DateTime, period: StackedBarPeriod, tz: &'static Tz)
    -> (Vec<StackedBarSeries>, Vec<StackedBarPoint>)
{
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let day_format = time::format_description::parse("[day] [month repr:short]").unwrap();
    let time_format = time::format_description::parse("[hour]:[minute]").unwrap();
    let month_format = time::format_description::parse("[month repr:short] [year]").unwrap();
    let long_month_format = time::format_description::parse("[month repr:long] [year]").unwrap();
    let session_format = time::format_description::parse("[weekday repr:short] [day] [month repr:short] [hour]:[minute]").unwrap();

    let series: Vec<StackedBarSeries> = sessions.iter()
        .enumerate()
        .map(|(index, session)| StackedBarSeries {
            label: format!(
                "{}{}",
                session.start.to_timezone(tz).format(&session_format).unwrap_or_default(),
                if session.is_public() { " (Public)" } else { "" },
            ),
            color: String::from(SESSION_COLORS[index % SESSION_COLORS.len()]),
        })
        .collect();

    if matches!(period, StackedBarPeriod::Raw | StackedBarPeriod::Rate) {
        let mut last_date = None;
        let points = sessions.iter()
            .enumerate()
            .map(|(index, session)| {
                let (session_start, session_end) = (session.start.to_timezone(tz), session.end.to_timezone(tz));
                let date = session_start.to_date();
                let tick_label = (last_date != Some(date)).then(|| session_start.format(&day_format).unwrap_or_default());
                last_date = Some(date);
                let mut values = vec![0.0; sessions.len()];
                values[index] = session.energy_kwh();
                StackedBarPoint {
                    detail: None,
                    start: session_start.unix_timestamp(),
                    end: session_end.unix_timestamp(),
                    tick_label,
                    tick_rank: 0,
                    period_label: format!(
                        "{} {}–{}",
                        session_start.format(&date_format).unwrap_or_default(),
                        session_start.format(&time_format).unwrap_or_default(),
                        session_end.format(&time_format).unwrap_or_default(),
                    ),
                    values,
                }
            })
            .collect();
        return (series, points);
    }

    let mut points = Vec::new();
    let mut bucket_start = period_bounds(start.to_timezone(tz).to_date(), period, tz).0;
    while bucket_start < *end {
        let (_, bucket_end) = period_bounds(bucket_start.to_date(), period, tz);
        let (from, to) = (bucket_start.unix_timestamp(), bucket_end.unix_timestamp());
        let values = sessions.iter()
            .map(|session| {
                let (session_start, session_end) = (session.start.unix_timestamp(), session.end.unix_timestamp());
                let share = if session_end > session_start {
                    (session_end.min(to) - session_start.max(from)).max(0) as f64 / (session_end - session_start) as f64
                }
                else if session_start >= from && session_start < to {
                    1.0
                }
                else {
                    0.0
                };
                session.energy_kwh() * share
            })
            .collect();
        let (tick_label, period_label) = match period {
            StackedBarPeriod::Week => (
                format!("Week of {}", bucket_start.format(&day_format).unwrap_or_default()),
                format!("Week of {}", bucket_start.format(&date_format).unwrap_or_default()),
            ),
            StackedBarPeriod::Month => (
                bucket_start.format(&month_format).unwrap_or_default(),
                bucket_start.format(&long_month_format).unwrap_or_default(),
            ),
            _ => (
                bucket_start.format(&day_format).unwrap_or_default(),
                bucket_start.format(&date_format).unwrap_or_default(),
            ),
        };
        points.push(StackedBarPoint {
            detail: None,
            start: from,
            end: to,
            tick_label: Some(tick_label),
            tick_rank: 0,
            period_label,
            values,
        });
        bucket_start = bucket_end;
    }
    (series, points)
}

#[cfg(test)]
mod tests {
    use bruce_graphql::types::DateTime;

    use super::{ChargingSession, ERROR, FULL, NO_CHARGE, PUBLIC, Problem, SMART, graph_points};
    use crate::octopus::decimal::Decimal;
    use crate::components::stacked_bar_graph::StackedBarPeriod;

    fn time(text: &str) -> DateTime {
        DateTime::from_unix_timestamp(time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap().unix_timestamp()).unwrap()
    }

    fn session(kind: &str, start: &str, end: &str, kwh: i64, problems: &[&str]) -> ChargingSession {
        ChargingSession {
            start: time(start),
            end: time(end),
            kind: String::from(kind),
            target: None,
            energy: Some((Decimal::new(kwh, 0), String::from("KILOWATT_HOUR"))),
            cost: None,
            state_of_charge_change: None,
            state_of_charge_final: None,
            dispatches: Vec::new(),
            problems: problems.iter().map(|cause| Problem::Error(cause.to_string())).collect(),
            location: None,
        }
    }

    #[test]
    fn a_session_is_shown_if_its_kind_is_unless_it_is_in_a_hidden_category() {
        let full = session("SMART", "2026-10-01T21:00:00Z", "2026-10-02T07:00:00Z", 30, &["FULL_CHARGE"]);
        let plugged_in = session("SMART", "2026-10-02T07:03:00Z", "2026-10-02T15:00:00Z", 0, &["NO_SCHEDULED_CHARGE"]);
        let error = session("SMART", "2026-10-03T21:00:00Z", "2026-10-04T07:00:00Z", 39, &["FAILURE_CAUSE_ERROR"]);
        let public = session("PUBLIC", "2026-10-05T15:00:00Z", "2026-10-05T15:30:00Z", 28, &[]);
        let shown = |hidden: &[&str]| {
            [&full, &plugged_in, &error, &public].iter()
                .filter(|session| session.shown(|category| !hidden.contains(&category)))
                .count()
        };
        assert_eq!(shown(&[]), 4);
        assert_eq!(shown(&[SMART]), 1);
        assert_eq!(shown(&[PUBLIC]), 3);
        assert_eq!(shown(&[FULL]), 3);
        assert_eq!(shown(&[NO_CHARGE]), 3);
        assert_eq!(shown(&[ERROR]), 3);
        // The other categories only hide sessions of a kind which is shown.
        assert_eq!(shown(&[SMART, FULL, NO_CHARGE, ERROR]), 1);
    }

    #[test]
    fn sessions_are_split_between_the_periods_they_cross_and_every_period_has_a_bar() {
        let london = time_tz::timezones::db::europe::LONDON;
        let sessions = [
            session("SMART", "2026-10-01T22:30:00Z", "2026-10-02T06:00:00Z", 10, &[]),
            session("SMART", "2026-10-02T23:30:00Z", "2026-10-03T06:00:00Z", 20, &[]),
            session("PUBLIC", "2026-10-03T12:00:00Z", "2026-10-03T12:30:00Z", 5, &[]),
        ];
        let sessions: Vec<&ChargingSession> = sessions.iter().collect();
        // 1 to 4 October BST.
        let (start, end) = (time("2026-09-30T23:00:00Z"), time("2026-10-04T23:00:00Z"));

        // 23:30 BST on the 1st to 07:00 on the 2nd is split, half an hour on the 1st, and 00:30 BST on the 3rd, 23:30
        // UTC on the 2nd, counts on the 3rd with the public session.
        let (series, days) = graph_points(&sessions, &start, &end, StackedBarPeriod::Day, london);
        assert_eq!(series.len(), 3);
        let close = |left: f64, right: f64| (left - right).abs() < 1e-9;
        let energies: Vec<f64> = days.iter().map(|point| point.values.iter().sum()).collect();
        assert_eq!(energies.len(), 4);
        for (energy, expected) in energies.iter().zip([10.0 * 0.5 / 7.5, 10.0 * 7.0 / 7.5, 25.0, 0.0]) {
            assert!(close(*energy, expected), "{energy} is not {expected}");
        }
        // The first session is split at midnight, by the time each side of it.
        assert!(close(days[0].values[0] + days[1].values[0], 10.0));

        let (_, raw) = graph_points(&sessions, &start, &end, StackedBarPeriod::Raw, london);
        assert_eq!(raw.len(), 3);
        assert_eq!(raw[2].values, [0.0, 0.0, 5.0]);

        let (_, months) = graph_points(&sessions, &start, &end, StackedBarPeriod::Month, london);
        assert_eq!(months.len(), 1);
        assert_eq!(months[0].values, [10.0, 20.0, 5.0]);
    }
}
