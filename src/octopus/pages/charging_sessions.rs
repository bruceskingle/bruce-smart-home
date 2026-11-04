//! The charging sessions of a charge point or vehicle, for a date range.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;
use crate::components::error_text::ErrorText;
use dioxus_free_icons::{Icon, icons::ld_icons::LdRefreshCw};
use time_tz::Tz;

use std::collections::BTreeSet;

use crate::components::date_range_picker::{DateRangePicker, RangeSelection};
use sparko_graphql::types::{Currency, DateTime};

use crate::components::category_filter::{CategoryFilter, CategoryFilterMenu, SOLID};
use crate::octopus::charging_session::{self, CATEGORIES, ChargingSession, energy_unit, readable};
use crate::data_set::CancelToken;
use crate::octopus::context::OctopusContext;
use crate::octopus::decimal::Decimal;
use crate::components::stacked_bar_graph::{BarClick, BarMenuItem, StackedBarGraph, StackedBarMode, StackedBarPeriod, day_bands, format_money};
use crate::octopus::trend_graph::{TrendGraph, TrendLegendItem, TrendLine, TrendSmoothing, cumulative_by_year, moving_average, range_years, year_positions};

/// The aggregations offered, sessions have no rates so there is no Rate.
const PERIODS: [StackedBarPeriod; 4] = [StackedBarPeriod::Raw, StackedBarPeriod::Day, StackedBarPeriod::Week, StackedBarPeriod::Month];

#[derive(Clone, Copy, PartialEq)]
enum SessionsView {
    Table,
    Graph,
    /// A line for each year, to compare years.
    Trend,
}

impl std::fmt::Display for SessionsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionsView::Table => write!(f, "Table"),
            SessionsView::Graph => write!(f, "Graph"),
            SessionsView::Trend => write!(f, "Trend"),
        }
    }
}

#[component]
/// battery_size is the size Octopus has for the EV's battery, if it has one.
pub fn ChargingSessions(account_id: String, device_id: String, device_name: String, battery_size: Option<Decimal>, billing_timezone: &'static Tz) -> Element {
    let context = use_context::<Arc<OctopusContext>>();
    let tz = billing_timezone;
    let mut range = use_signal(|| RangeSelection::current_month(tz));
    let mut view = use_signal(|| SessionsView::Table);
    let mut period = use_signal(|| StackedBarPeriod::Day);
    let mut smoothing = use_signal(|| TrendSmoothing::None);
    // Which categories of session are shown, in the table and the graphs, and the colours of the years in the trend.
    let filter = use_signal(CategoryFilter::default);
    // The range and period before each drill down or zoom in the graph, the latest last, for Zoom Out.
    let mut history = use_signal(Vec::<(RangeSelection, StackedBarPeriod)>::new);
    // A period whose months are fetched again however up to date they are, set by Reload, in case the cache is wrong.
    // The fetch does not depend on it, Reload restarts the fetch, so that it is only used once and later fetches, e.g.
    // of another range, use the cache.
    let mut reload = use_signal(|| None::<(DateTime, DateTime)>);
    // The latest progress message from the fetch.
    let progress = use_signal(String::new);
    // The cancel token of the fetch in progress, set by its Cancel button. Each fetch has a new one, so that cancelling
    // one does not cancel the next.
    let cancel = use_hook(|| Rc::new(RefCell::new(CancelToken::new())));
    // Set when the Cancel button is clicked, so that it shows the fetch is stopping, until the next fetch starts.
    let mut cancelling = use_signal(|| false);

    // Fetch the sessions, and again when Reload restarts it.
    let mut sessions_resource = use_resource({
        let context = context.clone();
        let account_id = account_id.clone();
        let device_id = device_id.clone();
        let cancel = cancel.clone();
        move || {
            let context = context.clone();
            let account_id = account_id.clone();
            let device_id = device_id.clone();
            let dates = range().to_date_times(tz);
            let reload_period = *reload.peek();
            let token = CancelToken::new();
            *cancel.borrow_mut() = token.clone();
            async move {
                let (start, end) = dates.map_err(|error| anyhow::anyhow!(error))?;
                cancelling.set(false);
                let monitor = move |message: String| { let mut progress = progress; progress.set(message) };
                let sessions = charging_session::fetch(&account_id, &device_id, &start, &end, reload_period, tz, &context.cache_config,
                    &*context.request_manager, &monitor, &token).await;
                if reload_period.is_some() {
                    reload.set(None);
                }
                sessions
            }
        }
    });
    // While the sessions are fetched there is a dialog over the page, with the sessions loaded before if there are any.
    let fetching = matches!(*sessions_resource.state().read(), UseResourceState::Pending);
    let fetching_title = if reload.peek().is_some() { "Reloading" } else { "Loading" };

    let (start, end) = range().to_date_times(tz).map_err(|error| anyhow::anyhow!(error))?;
    let categories: Vec<String> = CATEGORIES.iter().map(|category| category.to_string()).collect();
    let visible: Vec<(String, bool)> = categories.iter().cloned()
        .zip(filter.read().styles(&categories).into_iter().map(|style| style.visible))
        .collect();
    let is_visible = |category: &str| visible.iter().any(|(label, shown)| label == category && *shown);

    // The start and end of each session shown, to zoom to the sessions in a bar.
    let shown_times: Vec<(i64, i64)> = match &*sessions_resource.read() {
        Some(Ok(sessions)) => sessions.iter()
            .filter(|session| session.shown(is_visible))
            .map(|session| (session.start.unix_timestamp(), session.end.unix_timestamp()))
            .collect(),
        _ => Vec::new(),
    };

    // Double clicking a bar shows it in more detail: a month by week, a week by day, and a day as its sessions. The range
    // is from the start of the first session in the bar to the end of the last, so that a session split across the start
    // of the bar is included, as sessions are in a range by their start. It is in half hours, so the start is rounded
    // down and the end up.
    let on_drill = move |(bar_start, bar_end): (i64, i64)| {
        let next = match period() {
            StackedBarPeriod::Month => StackedBarPeriod::Week,
            StackedBarPeriod::Week => StackedBarPeriod::Day,
            StackedBarPeriod::Day => StackedBarPeriod::Raw,
            _ => return,
        };
        // The bar itself if it has no sessions.
        let (first_start, last_end) = shown_times.iter()
            .filter(|(start, end)| *start < bar_end && *end > bar_start)
            .fold(None, |extent: Option<(i64, i64)>, &(start, end)| Some(match extent {
                Some((first, last)) => (first.min(start), last.max(end)),
                None => (start, end),
            }))
            .unwrap_or((bar_start, bar_end));
        const HALF_HOUR: i64 = 1800;
        let zoom_start = first_start.div_euclid(HALF_HOUR) * HALF_HOUR;
        let zoom_end = (last_end + HALF_HOUR - 1).div_euclid(HALF_HOUR) * HALF_HOUR;
        if let Some(new_range) = RangeSelection::from_unix_timestamps(zoom_start, zoom_end, tz) {
            history.write().push((range(), period()));
            range.set(new_range);
            period.set(next);
        }
    };
    // Dragging across bars shows just those bars, with the same period.
    let on_zoom = move |(zoom_start, zoom_end): (i64, i64)| {
        if let Some(new_range) = RangeSelection::from_unix_timestamps(zoom_start, zoom_end, tz) {
            history.write().push((range(), period()));
            range.set(new_range);
        }
    };
    let (previous_range, next_range) = range().before_and_after(tz);
    let menu = vec![
        BarMenuItem {
            label: String::from("Zoom Out"),
            enabled: !history.read().is_empty(),
            action: EventHandler::new(move |_: BarClick| {
                if let Some((old_range, old_period)) = history.write().pop() {
                    range.set(old_range);
                    period.set(old_period);
                }
            }),
        },
        BarMenuItem {
            label: String::from("Previous"),
            enabled: previous_range.is_some(),
            action: EventHandler::new(move |_: BarClick| {
                if let Some(previous) = previous_range {
                    range.set(previous);
                }
            }),
        },
        BarMenuItem {
            label: String::from("Next"),
            enabled: next_range.is_some(),
            action: EventHandler::new(move |_: BarClick| {
                if let Some(next) = next_range {
                    range.set(next);
                }
            }),
        },
        BarMenuItem {
            label: String::from("Reload this bar"),
            enabled: true,
            action: EventHandler::new(move |click: BarClick| {
                if let (Ok(bar_start), Ok(bar_end)) = (DateTime::from_unix_timestamp(click.start), DateTime::from_unix_timestamp(click.end)) {
                    reload.set(Some((bar_start, bar_end)));
                    sessions_resource.restart();
                }
            }),
        },
    ];

    let content = match &*sessions_resource.read() {
        // The loading dialog is over the page.
        None => rsx! {},
        Some(Err(error)) => rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load charging sessions" } } },
        Some(Ok(sessions)) if sessions.is_empty() => rsx! { p { "No charging sessions in this period" } },
        Some(Ok(sessions)) => {
            let shown: Vec<&ChargingSession> = sessions.iter().filter(|session| session.shown(is_visible)).collect();
            match view() {
                SessionsView::Table if shown.is_empty() => rsx! { p { "No charging sessions of the categories shown" } },
                SessionsView::Table => sessions_table(&shown, battery_size, tz),
                SessionsView::Graph => {
                    let (series, points) = charging_session::graph_points(&shown, &start, &end, period(), tz);
                    // The bars of sessions are at their times, so the days are shown under them.
                    let x_bands = match (period(), points.first(), points.last()) {
                        (StackedBarPeriod::Raw, Some(first), Some(last)) => day_bands(first.start, last.end, tz),
                        _ => Vec::new(),
                    };
                    rsx! {
                        StackedBarGraph {
                            points,
                            // A series for each session, which the tooltip names.
                            series,
                            legend: false,
                            segment_tooltip: true,
                            x_bands,
                            mode: StackedBarMode::Units,
                            currency: Currency::NONE,
                            unit: String::from("kWh"),
                            on_drill: (period() != StackedBarPeriod::Raw).then(|| EventHandler::new(on_drill)),
                            on_zoom: EventHandler::new(on_zoom),
                            menu,
                        }
                    }
                },
                SessionsView::Trend => trend(&shown, &start, &end, period(), smoothing(), &filter.read(), tz),
            }
        },
    };

    rsx! {
        h1 { "Charging Sessions: {device_name}" }
        div { class: "date-range date-range-row",
            DateRangePicker { range, timezone: tz }
            span { class: "view-toggle",
                for option in [SessionsView::Table, SessionsView::Graph, SessionsView::Trend] {
                    button {
                        r#type: "button",
                        class: if view() == option { "active" },
                        onclick: move |_| view.set(option),
                        "{option}"
                    }
                }
            }
            if view() != SessionsView::Table {
                span { class: "view-toggle",
                    for option in PERIODS {
                        button {
                            r#type: "button",
                            class: if period() == option { "active" },
                            // Smoothing works from daily totals, so the period is not used.
                            disabled: view() == SessionsView::Trend && smoothing() != TrendSmoothing::None,
                            onclick: move |_| period.set(option),
                            // Each session is a bar.
                            if option == StackedBarPeriod::Raw { "Session" } else { "{option}" }
                        }
                    }
                }
            }
            if view() == SessionsView::Trend {
                span { class: "view-toggle", title: "Smooth the daily totals",
                    for option in TrendSmoothing::ALL {
                        button {
                            r#type: "button",
                            class: if smoothing() == option { "active" },
                            onclick: move |_| smoothing.set(option),
                            "{option}"
                        }
                    }
                }
            }
            CategoryFilterMenu {
                filter,
                categories,
                years: range_years(&start, &end),
                trend: view() == SessionsView::Trend,
                disabled: false,
                visibility_only: true,
            }
            button {
                r#type: "button",
                title: "Fetch the sessions shown from Octopus again, in case what has been saved is wrong",
                disabled: fetching,
                onclick: move |_| {
                    reload.set(Some((start, end)));
                    sessions_resource.restart();
                },
                Icon {
                    class: "inline-icon",
                    icon: LdRefreshCw,
                    width: 14,
                    height: 14,
                }
                " Reload"
            }
        }
        div { class: "reloadable charging-sessions-content",
            {content}
            if fetching {
                div { class: "reload-overlay",
                    div { class: "modal",
                        h3 { "{fetching_title}" }
                        div { "{progress}" }
                        div { class: "modal-buttons",
                            button {
                                r#type: "button",
                                disabled: cancelling(),
                                onclick: {
                                    let cancel = cancel.clone();
                                    move |_| {
                                        cancel.borrow().cancel();
                                        cancelling.set(true);
                                    }
                                },
                                if cancelling() { "Cancelling…" } else { "Cancel" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A line graph over one year of the energy added, with a line for each year from start to end. Smoothing other than
/// None is worked out from daily totals, so period is then not used.
fn trend(sessions: &[&ChargingSession], start: &DateTime, end: &DateTime, period: StackedBarPeriod, smoothing: TrendSmoothing,
    filter: &CategoryFilter, tz: &'static Tz) -> Element
{
    let period = if smoothing == TrendSmoothing::None { period } else { StackedBarPeriod::Day };
    let (_, points) = charging_session::graph_points(sessions, start, end, period, tz);
    let positions = year_positions(points.iter().map(|point| (point.start, point.end)), tz)?;
    let point_years: Vec<i32> = positions.iter().map(|(year, _)| *year).collect();
    let times: Vec<i64> = points.iter().map(|point| point.start).collect();
    let energies: Vec<f64> = points.iter().map(|point| point.values.iter().sum()).collect();
    let values = match smoothing {
        TrendSmoothing::None => energies,
        TrendSmoothing::MovingAverage(days) => moving_average(&times, &energies, days),
        TrendSmoothing::Cumulative => cumulative_by_year(&point_years, &energies),
    };
    let suffix = if matches!(smoothing, TrendSmoothing::MovingAverage(_)) { "/day" } else { "" };

    // A line for each year, in the year's colour.
    let years = range_years(start, end);
    let year_colors = filter.year_colors(&years);
    let shown_years: BTreeSet<i32> = point_years.iter().copied().collect();
    let mut lines = Vec::new();
    let mut legend = Vec::new();
    for (year, color) in years.iter().zip(&year_colors) {
        let year_points: Vec<(f64, f64)> = positions.iter()
            .zip(&values)
            .filter(|((point_year, _), _)| point_year == year)
            .map(|((_, position), value)| (*position, *value))
            .collect();
        if !year_points.is_empty() {
            lines.push(TrendLine { label: year.to_string(), color: color.clone(), dash: String::from(SOLID), points: year_points });
        }
        if shown_years.contains(year) {
            legend.push(TrendLegendItem { label: year.to_string(), color: color.clone(), dash: String::from(SOLID) });
        }
    }

    rsx! {
        TrendGraph {
            lines,
            legend,
            mode: StackedBarMode::Units,
            currency: Currency::NONE,
            unit: String::from("kWh"),
            suffix: String::from(suffix),
        }
    }
}

/// A value for a table cell, blank if there is none.
fn cell(value: Option<String>) -> String {
    value.unwrap_or_default()
}

/// An amount in the minor units of the currency code, e.g. pence, in its major units, e.g. £1.23
fn money(minor_units: &Decimal, currency_code: &str) -> String {
    let currency = Currency::from_code(currency_code).unwrap_or(Currency::NONE);
    format_money(minor_units.to_string().parse().unwrap_or_default(), currency)
}

/// A number to 2 decimal places, e.g. 3.70
fn two_dp(value: &Decimal) -> String {
    format!("{:.2}", value)
}

/// The average power in kW of energy added over a number of minutes, None if there was no time or energy.
fn average_kw(energy_kwh: Decimal, minutes: i64) -> Option<String> {
    if minutes > 0 && energy_kwh.is_positive() {
        Some(two_dp(&(energy_kwh * Decimal::from(60) / Decimal::from(minutes as i32))))
    }
    else {
        None
    }
}

/// The price per unit of energy of a cost in the minor units of its currency, e.g. pence per kWh, None if there was no
/// energy.
fn unit_price(cost: Decimal, energy: Decimal) -> Option<String> {
    energy.is_positive().then(|| two_dp(&(cost / energy)))
}

/// A duration as hours:minutes, or days:hours:minutes if it is a day or more.
fn long_duration(minutes: i64) -> String {
    if minutes >= 24 * 60 {
        format!("{}:{:02}:{:02}", minutes / (24 * 60), minutes / 60 % 24, minutes % 60)
    }
    else {
        format!("{}:{:02}", minutes / 60, minutes % 60)
    }
}


/// The battery size, in the units of the energy, which the percentages of sessions charging to a relative state of
/// charge are of, for when Octopus does not say what its configured size is. Octopus does not know the state of
/// charge of the EV, so its percentages are the energy added as a share of the configured size. The middle value of
/// the sessions, in case any are odd.
fn implied_battery_size(sessions: &[&ChargingSession]) -> Option<Decimal> {
    let mut sizes: Vec<Decimal> = sessions.iter()
        .filter(|session| session.target.as_deref() == Some("RELATIVE_STATE_OF_CHARGE"))
        .filter_map(|session| match (&session.energy, session.state_of_charge_final) {
            (Some((energy, _)), Some(percent)) if energy.is_positive() && percent.is_positive() => Some(*energy * Decimal::from(100) / percent),
            _ => None,
        })
        .collect();
    sizes.sort();
    sizes.get(sizes.len() / 2).map(|size| size.round_dp(0))
}

fn sessions_table(sessions: &[&ChargingSession], battery_size: Option<Decimal>, tz: &'static Tz) -> Element {
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let time_only = time::format_description::parse("[hour]:[minute]").unwrap();
    let local = |time: &sparko_graphql::types::DateTime| time.to_timezone(tz);
    let minutes = |session: &ChargingSession| (*session.end - *session.start).whole_minutes();
    let duration = |minutes: i64| format!("{}:{:02}", minutes / 60, minutes % 60);

    // Columns with nothing in them are left out.
    let has_location = sessions.iter().any(|session| session.location.is_some());
    let has_cost = sessions.iter().any(|session| session.cost.is_some());
    let has_percent = sessions.iter().any(|session| session.state_of_charge_final.is_some());
    let has_dispatches = sessions.iter().any(|session| !session.dispatches.is_empty());
    // What the percentages are of, the battery size configured in Octopus, or if it does not say, the size the
    // percentages imply.
    let configured_size = battery_size
        .map(|size| format!("{size} kWh"))
        .or_else(|| implied_battery_size(sessions).map(|size| format!("about {size} kWh")));
    let energy_unit = sessions.iter().find_map(|session| session.energy.as_ref().map(|(_, unit)| energy_unit(unit))).unwrap_or_default();
    let currency = sessions.iter().find_map(|session| session.cost.as_ref().map(|(_, currency)| currency.clone())).unwrap_or_default();

    // The start and end date and time of each session, the dates are blank where they are the same as the one before,
    // the end's is the start's and the start's is the start of the session before.
    let mut previous_date = String::new();
    let times: Vec<(String, String, String, String)> = sessions.iter()
        .map(|session| {
            let (start, end) = (local(&session.start), local(&session.end));
            let start_date = start.format(&date_format).unwrap_or_default();
            let end_date = end.format(&date_format).unwrap_or_default();
            let shown_start_date = if start_date == previous_date { String::new() } else { start_date.clone() };
            let shown_end_date = if end_date == start_date { String::new() } else { end_date };
            previous_date = start_date;
            (shown_start_date, start.format(&time_only).unwrap_or_default(), shown_end_date, end.format(&time_only).unwrap_or_default())
        })
        .collect();

    // The totals are of the sessions which charged.
    let charged: Vec<&ChargingSession> = sessions.iter().copied().filter(|session| !session.plugged_in_only()).collect();
    let plugged_in = sessions.len() - charged.len();
    let total_energy = charged.iter().filter_map(|session| session.energy.as_ref().map(|(value, _)| *value))
        .fold(Decimal::ZERO, |total, value| total + value);
    let total_cost = charged.iter().filter_map(|session| session.cost.as_ref().map(|(value, _)| *value))
        .fold(Decimal::ZERO, |total, value| total + value);
    let total_minutes: i64 = charged.iter().map(|session| minutes(session)).sum();
    // The average price is of the sessions with a cost, e.g. public charging, as smart charging has none.
    let costed_energy = charged.iter()
        .filter(|session| session.cost.is_some())
        .filter_map(|session| session.energy.as_ref().map(|(value, _)| *value))
        .fold(Decimal::ZERO, |total, value| total + value);
    let count = if plugged_in > 0 {
        format!("{} charging, {plugged_in} plugged in without charging", charged.len())
    }
    else {
        format!("{} charging", charged.len())
    };

    rsx! {
        table { class: "charging-sessions",
            tr { class: "header",
                th { colspan: 2, "Start" }
                th { colspan: 2, "End" }
                th { "Duration" br {} "(h:mm)" }
                th { "Type" }
                if has_location {
                    th { "Where" }
                }
                th { class: "quantity", "Energy" br {} "({energy_unit})" }
                th { class: "quantity", "Avg Power" br {} "(kW)" }
                if has_cost {
                    th { "Cost" }
                    th { title: "The cost divided by the energy added", class: "quantity", "Price" br {} "(p/{energy_unit})" }
                }
                if has_percent {
                    th { title: "The energy added as a percentage of the battery size configured in Octopus, not of the EV's actual battery", class: "quantity", "% Battery" }
                }
                if has_dispatches {
                    th { "Dispatches" }
                }
                th { "Notes" }
            }
            for (session, (start_date, start_time, end_date, end_time)) in sessions.iter().zip(times) {
                tr { class: if session.plugged_in_only() { "plugged-in" },
                    td { "{start_date}" }
                    td { "{start_time}" }
                    td { "{end_date}" }
                    td { "{end_time}" }
                    td { {duration(minutes(session))} }
                    td { {readable(&session.kind)} }
                    if has_location {
                        td { {cell(session.location.clone())} }
                    }
                    td { class: "numeric", {cell(session.energy.as_ref().map(|(value, _)| two_dp(value)))} }
                    // Worked out from the energy and duration, over the whole session.
                    td { class: "numeric derived", {cell(session.energy.as_ref().and_then(|(value, _)| average_kw(*value, minutes(session))))} }
                    if has_cost {
                        td { class: "numeric", {cell(session.cost.as_ref().map(|(value, currency)| money(value, currency)))} }
                        td { class: "numeric derived", {cell(session.cost.as_ref().zip(session.energy.as_ref())
                            .and_then(|((cost, _), (energy, _))| unit_price(*cost, *energy)))} }
                    }
                    if has_percent {
                        td { class: "numeric", {cell(session.state_of_charge_final.map(|value| two_dp(&value)))} }
                    }
                    if has_dispatches {
                        td {
                            for dispatch in session.dispatches.iter() {
                                div {
                                    {format!(
                                        "{}-{} {} {}",
                                        local(&dispatch.start).format(&time_only).unwrap_or_default(),
                                        local(&dispatch.end).format(&time_only).unwrap_or_default(),
                                        readable(&dispatch.kind),
                                        dispatch.energy_kwh.map(|kwh| format!("{kwh} kWh")).unwrap_or_default(),
                                    )}
                                }
                            }
                        }
                    }
                    td {
                        if session.plugged_in_only() {
                            div { "Plugged in, no charge" }
                        }
                        for problem in session.problems.iter() {
                            div { class: problem.class(), {problem.description()} }
                        }
                    }
                }
            }
            tr { class: "total",
                td { "Total" }
                td { class: "derived", colspan: 3, "{count}" }
                td { class: "derived", {long_duration(total_minutes)} }
                td {}
                if has_location {
                    td {}
                }
                td { class: "numeric derived", {two_dp(&total_energy)} }
                td { class: "numeric derived", {cell(average_kw(total_energy, total_minutes))} }
                if has_cost {
                    td { class: "numeric derived", {money(&total_cost, &currency)} }
                    td { class: "numeric derived", {cell(unit_price(total_cost, costed_energy))} }
                }
                if has_percent {
                    td {}
                }
                if has_dispatches {
                    td {}
                }
                td {}
            }
        }
        if has_percent {
            p { class: "note",
                "% of Configured Battery is the energy added as a percentage of the battery size configured in Octopus"
                if let Some(size) = configured_size {
                    " ({size})"
                }
                ". Octopus does not know which EV was charging or how full it was, so this is not the EV's charge level, and is only a percentage of its battery if it is the EV the size is configured for."
            }
        }
    }
}
