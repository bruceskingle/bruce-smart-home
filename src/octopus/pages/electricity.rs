//! The electricity imported and exported, as a table or a graph with import above zero split into the rates it is
//! billed at, and export below zero, combined from the meters' half hourly readings, the Home Mini's readings every
//! 10 seconds, and the Tesla gateway's registers, which split it into what it was used for, see electricity.rs, with
//! Octopus's split of the car's import as a line over the bars, to compare.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::anyhow;
use dioxus::prelude::*;
use crate::components::error_text::ErrorText;
use dioxus_free_icons::{Icon, icons::ld_icons::LdRefreshCw};
use sparko_graphql::types::{Currency, DateTime};
use time_tz::{OffsetDateTimeExt, Tz};

use crate::components::date_range_picker::{DateRangePicker, LocalSlot, RangeSelection};
use crate::components::export::{ExportButton, ExportTable, ExportValue, export_file_name};
use crate::components::toggle_button::ToggleButton;
use crate::data_set::CancelToken;
use crate::octopus::category_filter::{CategoryFilter, CategoryFilterMenu, CategoryValues};
use crate::octopus::context::OctopusContext;
use crate::octopus::electricity::{self, CheapTimes, EXPORT_LABEL, MEASURED_EV_LABEL, OCTOPUS_EV_LABEL, is_intelligent, Electricity, ElectricityPoint, Estimate, EvComparison, MeterHalfHour};
use crate::octopus::graphql::ElectricityDirection;
use crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType;
use crate::octopus::measurements::MeasurementsDataSet;
use crate::octopus::meter_point::MeterPointDataSet;
use crate::octopus::property::PropertyDataSet;
use crate::octopus::smart_device::SmartDeviceDataSet;
use crate::octopus::stacked_bar_graph::{BarClick, BarMenuItem, MAX_BARS, StackedBarGraph, StackedBarLine, StackedBarMode, StackedBarPoint, StackedBarSeries, day_bands, time_tick_label};
use crate::octopus::telemetry::{self, TelemetryPeriod, TelemetryReading};
use super::measurements::graphql_error_descriptions;

/// The most rows in the table.
const MAX_SHOWN: usize = 9000;
const HALF_HOUR: i64 = 1800;

#[derive(Clone, Copy, PartialEq)]
enum ElectricityView {
    Table,
    Graph,
}

impl std::fmt::Display for ElectricityView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElectricityView::Table => write!(f, "Table"),
            ElectricityView::Graph => write!(f, "Graph"),
        }
    }
}

/// What the graph shows.
#[derive(Clone, Copy, PartialEq)]
enum ElectricityMode {
    /// In kWh.
    Energy,
    /// The average in kW.
    Power,
}

impl ElectricityMode {
    const ALL: [ElectricityMode; 2] = [ElectricityMode::Energy, ElectricityMode::Power];

    /// The import of each of labels and the export of a point as shown, the energy itself in kWh, or the average power
    /// in kW. For power, a point with the Home Mini's average power, which is steadier than its energy, see
    /// ElectricityPoint::demand, uses it for the house's labels: the car charges at a constant power, so its labels are
    /// from their energy, and the house's share the rest of the Home Mini's power in proportion to their energy.
    fn values(&self, point: &ElectricityPoint, labels: &[String]) -> (Vec<f64>, f64) {
        let hours = (point.end - point.start).max(1) as f64 / 3600.0;
        match (self, point.demand) {
            (ElectricityMode::Energy, _) => (point.import.clone(), point.export),
            (ElectricityMode::Power, None) => (point.import.iter().map(|kwh| kwh / hours).collect(), point.export / hours),
            (ElectricityMode::Power, Some(demand)) => {
                let is_ev = |index: usize| labels[index].starts_with("EV");
                let ev_kw: f64 = (0..labels.len()).filter(|index| is_ev(*index)).map(|index| point.import[index] / hours).sum();
                let home_kwh: f64 = (0..labels.len()).filter(|index| !is_ev(*index)).map(|index| point.import[index]).sum();
                let home_kw = (demand.max(0.0) / 1000.0 - ev_kw).max(0.0);
                let import = (0..labels.len())
                    .map(|index| match (is_ev(index), home_kwh > 0.0) {
                        (true, _) => point.import[index] / hours,
                        (false, true) => home_kw * point.import[index] / home_kwh,
                        (false, false) => 0.0,
                    })
                    .collect();
                (import, (-demand).max(0.0) / 1000.0)
            },
        }
    }

    fn unit(&self) -> &'static str {
        match self {
            ElectricityMode::Energy => "kWh",
            ElectricityMode::Power => "kW",
        }
    }
}

impl std::fmt::Display for ElectricityMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElectricityMode::Energy => write!(f, "Energy"),
            ElectricityMode::Power => write!(f, "Power"),
        }
    }
}

/// Where a point's energy is from, for its tooltip and the table, e.g. Home Mini and Tesla.
fn source(point: &ElectricityPoint) -> String {
    let sources: Vec<&str> = [(point.home_mini, "Home Mini"), (point.tesla, "Tesla"), (point.meter, "meter")].into_iter()
        .filter_map(|(is, name)| is.then_some(name))
        .collect();
    match sources.as_slice() {
        [] => String::new(),
        [one] => String::from(*one),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The car's import measured and as Octopus split it, in kWh, over the points' half hours which have both, see
/// ElectricityPoint::ev_comparison, None if none have.
fn ev_comparison(points: &[ElectricityPoint]) -> Option<EvComparison> {
    points.iter().filter_map(|point| point.ev_comparison).reduce(|total, more| total + more)
}

/// What Octopus's split of the car's import in the half hours outside the cheap times did to the bill, compared with
/// the measured split, see EvComparison::octopus_outside: the car's import there is at the low rate, and the house's at
/// the standard one, so each kWh more of the car's is the difference between them less. rates are the standard and low
/// rates in pence a kWh, None if they are not known.
fn outside_cheap_times(comparison: &EvComparison, rates: Option<(f64, f64)>) -> String {
    let more = comparison.octopus_outside - comparison.measured_outside;
    if comparison.octopus_outside.abs() < 0.005 && comparison.measured_outside.abs() < 0.005 {
        return String::from("All of it was in the cheap times, where the split does not change what it costs.");
    }
    let worth = rates.map(|(standard, low)| {
        let pounds = more.abs() * (standard - low) / 100.0;
        format!(", worth £{pounds:.2} {}", if more >= 0.0 { "less on the bill than the measured split" } else { "more on the bill than the measured split" })
    }).unwrap_or_default();
    format!("Outside the cheap times, where the split decides the rate: measured {:.2} kWh, Octopus {:.2} kWh, so Octopus put {:.2} kWh {} at the low rate{worth}.",
        comparison.measured_outside, comparison.octopus_outside, more.abs(), if more >= 0.0 { "more" } else { "less" })
}

/// The range of half hours containing the times from start to end, as unix timestamps, as ranges are in half hours.
fn half_hour_range(start: i64, end: i64, tz: &'static Tz) -> Option<RangeSelection> {
    let start = start.div_euclid(HALF_HOUR) * HALF_HOUR;
    let end = (end + HALF_HOUR - 1).div_euclid(HALF_HOUR) * HALF_HOUR;
    RangeSelection::from_unix_timestamps(start, end.max(start + HALF_HOUR), tz)
}

#[derive(Props, Clone, PartialEq)]
pub struct ElectricityPageProps {
    path: Vec<String>,
}

/// Finds the account's import and export meter points and its Home Mini's meter, then shows their electricity.
#[component]
pub fn ElectricityPage(props: ElectricityPageProps) -> Element {
    let _ = props.path;
    let context = use_context::<Arc<OctopusContext>>();
    let meter_points_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move { MeterPointDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await }
        }
    });
    let properties_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move { PropertyDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await }
        }
    });
    // The charger, for the power at which the car is charging, see Estimate.
    let devices_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move { SmartDeviceDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await }
        }
    });

    // The import meter point's cheap times, from the time of use scheme of its current agreement.
    let mut cheap = None;
    let mut intelligent = false;
    // Its standard and low rates, in pence a kWh, for what Octopus's split of the car's import is worth, if it is a day
    // and night tariff, as Intelligent Octopus Go is.
    let mut rates = None;
    let (meter_points, properties) = match (&*meter_points_resource.read(), &*properties_resource.read()) {
        (Some(Err(error)), _) => return rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the meter points" } } },
        (_, Some(Err(error))) => return rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the properties" } } },
        (Some(Ok(meter_points)), Some(Ok(properties))) => (
            meter_points.meter_points.iter()
                .map(|meter_point| {
                    if meter_point.direction_ == Some(ElectricityDirection::Import) {
                        let timeslots: Vec<(String, String, String)> = meter_point.agreements_.iter()
                            .filter_map(|agreement| agreement.time_of_use_scheme_.as_ref())
                            .flat_map(|scheme| scheme.timeslots_.iter())
                            .map(|slot| (
                                slot.timeslot_.clone(),
                                slot.active_from_.as_str().unwrap_or_default().to_string(),
                                slot.active_to_.as_str().unwrap_or_default().to_string(),
                            ))
                            .collect();
                        cheap = CheapTimes::from_timeslots(&timeslots);
                        intelligent = meter_point.agreements_.iter()
                            .filter_map(|agreement| agreement.tariff_.as_ref())
                            .filter_map(|tariff| match tariff {
                                ElectricityTariffType::StandardTariff(tariff) => tariff.product_code_.as_deref(),
                                ElectricityTariffType::DayNightTariff(tariff) => tariff.product_code_.as_deref(),
                                ElectricityTariffType::ThreeRateTariff(tariff) => tariff.product_code_.as_deref(),
                                ElectricityTariffType::FourRateEvTariff(tariff) => tariff.product_code_.as_deref(),
                                ElectricityTariffType::HalfHourlyTariff(tariff) => tariff.product_code_.as_deref(),
                                ElectricityTariffType::PrepayTariff(tariff) => tariff.product_code_.as_deref(),
                            })
                            .any(is_intelligent);
                        rates = meter_point.agreements_.iter()
                            .filter_map(|agreement| agreement.tariff_.as_ref())
                            .find_map(|tariff| match tariff {
                                ElectricityTariffType::DayNightTariff(tariff) => tariff.day_rate_.zip(tariff.night_rate_),
                                _ => None,
                            });
                    }
                    (meter_point.mpan_.clone(), meter_point.direction_.clone())
                })
                .collect::<Vec<_>>(),
            properties.property_map.values()
                .flat_map(|property| property.smart_device_networks_.iter())
                .flat_map(|network| network.smart_devices_.iter())
                .find(|device| device.type_.to_string().trim_matches('"') == "ESME")
                .map(|device| device.device_id_.clone()),
        ),
        _ => return rsx! { div { "Loading the meter points..." } },
    };
    let esme_id = properties;
    let mpan = |direction: ElectricityDirection| meter_points.iter()
        .find(|(_, meter_direction)| meter_direction.as_ref() == Some(&direction))
        .map(|(mpan, _)| mpan.clone());
    let Some(import_mpan) = mpan(ElectricityDirection::Import) else {
        return rsx! { div { "There is no electricity import meter point on this account" } };
    };
    // Without the charger's power, the rate is still estimated from the cheap times, but not while charging outside them,
    // and the car's share of the import is not placed where it was charging.
    let charger_watts = match &*devices_resource.read() {
        Some(Ok(devices)) => devices.devices.iter()
            .filter(|device| device.charging.is_some())
            .filter_map(|device| device.power.as_ref().and_then(|power| power.to_string().parse::<f64>().ok()))
            .map(|kw| kw * 1000.0)
            .reduce(f64::min),
        _ => None,
    };
    let estimate = Estimate { cheap, intelligent, charger_watts };
    rsx! {
        ElectricityGraph {
            key: "{import_mpan}",
            account_id: context.account_id.clone(),
            import_mpan,
            export_mpan: mpan(ElectricityDirection::Export),
            esme_id,
            estimate,
            rates,
            billing_timezone: context.billing_timezone,
        }
    }
}

/// The readings for the range shown, and why the Home Mini's could not be fetched, if they could not, as the meters'
/// are still worth showing.
struct Readings {
    import: Vec<MeterHalfHour>,
    export: Vec<MeterHalfHour>,
    home_mini: Vec<TelemetryReading>,
    home_mini_error: Option<String>,
    /// The car's charging power in W, measured from the meter's readings, if they have it charging.
    charging_watts: Option<f64>,
    /// The Tesla gateway's registers, as a capture device saved them, see tesla/snapshot.rs, to split the import.
    tesla: Vec<crate::tesla::snapshot::Registers>,
}

#[component]
fn ElectricityGraph(account_id: String, import_mpan: String, export_mpan: Option<String>, esme_id: Option<String>,
    estimate: Estimate, rates: Option<(f64, f64)>, billing_timezone: &'static Tz) -> Element
{
    let cheap_known = estimate.cheap.is_some();
    let context = use_context::<Arc<OctopusContext>>();
    let tz = billing_timezone;
    // The last 7 days, so that both the meters' readings and the Home Mini's more recent ones are shown.
    let mut range = use_signal(|| {
        let today = crate::clock::today(tz);
        RangeSelection { start: LocalSlot { date: today - time::Duration::days(6), slot: 0 }, end: None }
    });
    let mut view = use_signal(|| ElectricityView::Graph);
    let mut period = use_signal(|| TelemetryPeriod::HalfHour);
    let mut mode = use_signal(|| ElectricityMode::Energy);
    let filter = use_signal(CategoryFilter::default);
    // The range and period before each drill down or zoom in the graph, the latest last, for Zoom Out.
    let mut history = use_signal(Vec::<(RangeSelection, TelemetryPeriod)>::new);
    // A period which is fetched again however up to date it is, set by Reload, see Telemetry.
    let mut reload = use_signal(|| None::<(DateTime, DateTime)>);
    let progress = use_signal(String::new);
    let cancel = use_hook(|| Rc::new(RefCell::new(CancelToken::new())));
    let mut cancelling = use_signal(|| false);

    let mut readings_resource = use_resource({
        let context = context.clone();
        let cancel = cancel.clone();
        move || {
            let context = context.clone();
            let account_id = account_id.clone();
            let import_mpan = import_mpan.clone();
            let export_mpan = export_mpan.clone();
            let esme_id = esme_id.clone();
            let dates = range().to_date_times(tz);
            let reload_period = *reload.peek();
            let token = CancelToken::new();
            *cancel.borrow_mut() = token.clone();
            async move {
                cancelling.set(false);
                let (start, end) = dates.map_err(|error| anyhow!(error))?;
                // The progress of each fetch, with what it is fetching.
                let monitor = |what: &'static str| move |message: String| { let mut progress = progress; progress.set(format!("{what}: {message}")) };
                let import = MeasurementsDataSet::new(&account_id, &import_mpan, false, &start, &end, reload_period, tz,
                    &monitor("Import meter"), &token, &context.cache_config, &*context.request_manager).await?;
                let export = match &export_mpan {
                    Some(export_mpan) => Some(MeasurementsDataSet::new(&account_id, export_mpan, true, &start, &end, reload_period, tz,
                        &monitor("Export meter"), &token, &context.cache_config, &*context.request_manager).await?),
                    None => None,
                };
                let (home_mini, home_mini_error) = match &esme_id {
                    Some(esme_id) => match telemetry::fetch(&account_id, esme_id, &start, &end, reload_period, tz, &context.cache_config,
                        &*context.request_manager, &monitor("Home Mini"), &token).await
                    {
                        Ok(readings) => (readings, None),
                        Err(error) => (Vec::new(), Some(match graphql_error_descriptions(&error) {
                            Some(descriptions) => descriptions.join(", "),
                            None => error.to_string(),
                        })),
                    },
                    None => (Vec::new(), None),
                };
                if reload_period.is_some() {
                    reload.set(None);
                }
                // The Tesla module's, of the same profile, if it has any, read from its files, which are large.
                let tesla_cache = context.cache_config.sibling("tesla");
                let (tesla_start, tesla_end) = (start.unix_timestamp() - 1800, end.unix_timestamp() + 1800);
                let tesla = tokio::task::spawn_blocking(move || tesla_cache
                        .map(|cache| crate::tesla::snapshot::registers(&cache, tesla_start, tesla_end))
                        .unwrap_or_default())
                    .await
                    .unwrap_or_default();
                // The half hours either side too, for whether the car was charging in them, see electricity::combine.
                let (before, after) = (DateTime::from_unix_timestamp(start.unix_timestamp() - 1800)?, DateTime::from_unix_timestamp(end.unix_timestamp() + 1800)?);
                // The car's charging power, from the most the meter has given the car in a half hour, which is what it
                // actually charges at, e.g. 7.1 kW for a charger rated at 7.4 kW.
                let charging_watts = import.half_hours(&DateTime::from_unix_timestamp(0)?, &DateTime::from_unix_timestamp(i32::MAX as i64)?).iter()
                    .map(|half_hour| half_hour.units.iter().filter(|(label, _)| label.starts_with("EV")).map(|(_, units)| units).sum::<f64>())
                    .fold(0.0, f64::max) * 2000.0;
                anyhow::Ok(Arc::new(Readings {
                    import: import.half_hours(&before, &after),
                    export: export.map(|export| export.half_hours(&start, &end)).unwrap_or_default(),
                    charging_watts: (charging_watts >= 1000.0).then_some(charging_watts),
                    home_mini,
                    home_mini_error,
                    tesla,
                }))
            }
        }
    });
    let fetching = matches!(*readings_resource.state().read(), UseResourceState::Pending);
    let fetching_title = if reload.peek().is_some() { "Reloading" } else { "Loading" };

    let (start, end) = range().to_date_times(tz).map_err(|error| anyhow!(error))?;
    // As on the Home Mini page, periods which would be too many bars or rows can not be chosen, and the one shown is the
    // chosen one or the shortest longer one which fits.
    let most_shown = if view() == ElectricityView::Graph { MAX_BARS } else { MAX_SHOWN };
    let fits = |option: TelemetryPeriod| option.seconds()
        .is_none_or(|seconds| ((end.unix_timestamp() - start.unix_timestamp()) / seconds) as usize <= most_shown);
    let shown_period = TelemetryPeriod::ALL.into_iter()
        .find(|option| *option >= period() && fits(*option))
        .unwrap_or(TelemetryPeriod::Month);
    let readings: Option<Arc<Readings>> = match &*readings_resource.read() {
        Some(Ok(readings)) => Some(readings.clone()),
        _ => None,
    };
    let electricity = readings.as_ref().map(|readings| {
        // The charging power measured from the meter's readings, rather than the charger's rating, if it can be.
        let estimate = Estimate { charger_watts: readings.charging_watts.or(estimate.charger_watts), ..estimate.clone() };
        electricity::combine(&readings.import, &readings.export, &readings.home_mini, &readings.tesla, start.unix_timestamp(),
            end.unix_timestamp(), shown_period, &estimate, tz)
    });
    let comparison = electricity.as_ref().and_then(|electricity| ev_comparison(&electricity.points));
    // The import labels and export, and Octopus's split of the car's import if there is any to compare, for the
    // categories menu.
    let categories: Vec<String> = electricity.as_ref()
        .map(|electricity| electricity.labels.iter().cloned()
            .chain([String::from(EXPORT_LABEL)])
            .chain(comparison.map(|_| String::from(OCTOPUS_EV_LABEL)))
            .collect())
        .unwrap_or_default();

    let on_drill = move |(bar_start, bar_end): (i64, i64)| {
        let Some(next) = shown_period.shorter() else {
            return;
        };
        if let Some(new_range) = half_hour_range(bar_start, bar_end, tz) {
            history.write().push((range(), shown_period));
            range.set(new_range);
            period.set(next);
        }
    };
    let on_zoom = move |(zoom_start, zoom_end): (i64, i64)| {
        if let Some(new_range) = half_hour_range(zoom_start, zoom_end, tz).filter(|new_range| *new_range != range()) {
            history.write().push((range(), shown_period));
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
                    readings_resource.restart();
                }
            }),
        },
    ];

    let export_electricity = electricity.clone();
    let export_table = move |_: ()| export_electricity.as_ref().map(|electricity| points_export(electricity, tz));
    let export_name = export_file_name("electricity", "all", &start, &end, &shown_period.to_string());

    let home_mini_error = readings.as_ref().and_then(|readings| readings.home_mini_error.clone());
    let content = match (&*readings_resource.read(), &electricity) {
        (None, _) => rsx! {},
        (Some(Err(error)), _) => match graphql_error_descriptions(error) {
            Some(descriptions) => rsx! {
                div { class: "error",
                    "Octopus did not provide the readings:"
                    ul {
                        for description in descriptions {
                            li { "{description}" }
                        }
                    }
                }
            },
            None => rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the readings" } } },
        },
        (Some(Ok(_)), Some(electricity)) if electricity.points.is_empty() => rsx! { p { "No readings in this period" } },
        (Some(Ok(_)), Some(electricity)) => rsx! {
            match view() {
                ElectricityView::Table => electricity_table(electricity, tz),
                ElectricityView::Graph => electricity_graph(electricity, &start, &end, shown_period, mode(), &filter.read(), tz, on_drill, on_zoom, menu),
            }
            if let Some(comparison) = comparison {
                p { class: "derived",
                    "The car, where it was measured and Octopus has split the half hours: measured {comparison.measured:.2} kWh, Octopus {comparison.octopus:.2} kWh, "
                    {format!("difference {:+.2} kWh ({:+.1}%)", comparison.octopus - comparison.measured,
                        if comparison.measured > 0.0 { (comparison.octopus - comparison.measured) / comparison.measured * 100.0 } else { 0.0 })}
                }
                if cheap_known {
                    p { class: "derived", {outside_cheap_times(&comparison, rates)} }
                }
            }
        },
        (Some(Ok(_)), None) => rsx! {},
    };

    rsx! {
        h1 { "Electricity" }
        if let Some(error) = home_mini_error {
            p { class: "invalid", "The Home Mini's readings could not be fetched, only the meters' are shown: {error}" }
        }
        div { class: "date-range date-range-row",
            DateRangePicker { range, timezone: tz }
            span { class: "view-toggle",
                for option in [ElectricityView::Table, ElectricityView::Graph] {
                    button {
                        r#type: "button",
                        class: if view() == option { "active" },
                        onclick: move |_| view.set(option),
                        "{option}"
                    }
                }
            }
            if view() == ElectricityView::Graph {
                span { class: "view-toggle",
                    for option in ElectricityMode::ALL {
                        button {
                            r#type: "button",
                            class: if mode() == option { "active" },
                            onclick: move |_| mode.set(option),
                            "{option}"
                        }
                    }
                }
            }
            span { class: "view-toggle",
                for option in TelemetryPeriod::ALL {
                    ToggleButton {
                        label: option.to_string(),
                        active: shown_period == option,
                        unavailable: (!fits(option)).then(|| String::from("Zoom in to see more")),
                        onclick: move |_| period.set(option),
                    }
                }
            }
            if view() == ElectricityView::Graph {
                CategoryFilterMenu {
                    filter,
                    categories: categories.clone(),
                    years: Vec::<i32>::new(),
                    trend: false,
                    disabled: categories.is_empty(),
                }
            }
            button {
                r#type: "button",
                title: "Fetch the readings shown from Octopus again",
                disabled: fetching,
                onclick: move |_| {
                    reload.set(Some((start, end)));
                    readings_resource.restart();
                },
                Icon {
                    class: "inline-icon",
                    icon: LdRefreshCw,
                    width: 14,
                    height: 14,
                }
                " Reload"
            }
            ExportButton {
                default_name: export_name,
                disabled: electricity.is_none(),
                table: export_table,
            }
        }
        div { class: "reloadable telemetry-content",
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

/// A message instead of more rows than can be shown.
fn too_many(count: usize) -> Element {
    rsx! {
        p { "That is {count} periods, too many to show. Choose a longer period or a shorter range." }
    }
}

/// The points as a table, for export.
fn points_export(electricity: &Electricity, tz: &'static Tz) -> ExportTable {
    let mut columns = vec![String::from("Start"), String::from("End")];
    columns.extend(electricity.labels.iter().map(|label| format!("{label} (kWh)")));
    columns.extend([format!("{EXPORT_LABEL} (kWh)"), format!("{OCTOPUS_EV_LABEL} (kWh)"), String::from("Source")]);
    ExportTable {
        columns,
        rows: electricity.points.iter()
            .map(|point| {
                let mut row = vec![ExportValue::unix_time(point.start, tz), ExportValue::unix_time(point.end, tz)];
                row.extend(point.import.iter().map(|kwh| ExportValue::number(*kwh)));
                row.push(ExportValue::number(point.export));
                row.push(point.ev_comparison.map(|comparison| ExportValue::number(comparison.octopus)).unwrap_or(ExportValue::Text(String::new())));
                row.push(ExportValue::Text(source(point)));
                row
            })
            .collect(),
    }
}

/// A row for each period, with the import of each label, the export and where they are from.
fn electricity_table(electricity: &Electricity, tz: &'static Tz) -> Element {
    if electricity.points.len() > MAX_SHOWN {
        return too_many(electricity.points.len());
    }
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let time_format = time::format_description::parse("[hour]:[minute]:[second]").unwrap();
    let local = |timestamp: i64| time::OffsetDateTime::from_unix_timestamp(timestamp).map(|time| time.to_timezone(tz)).ok();

    let mut previous_date = String::new();
    let rows: Vec<(String, String, String, Vec<String>, String, String, String)> = electricity.points.iter()
        .map(|point| {
            let (start, end) = (local(point.start), local(point.end));
            let date = start.map(|start| start.format(&date_format).unwrap_or_default()).unwrap_or_default();
            let shown_date = if date == previous_date { String::new() } else { date.clone() };
            previous_date = date;
            (
                shown_date,
                start.map(|start| start.format(&time_format).unwrap_or_default()).unwrap_or_default(),
                end.map(|end| end.format(&time_format).unwrap_or_default()).unwrap_or_default(),
                point.import.iter().map(|kwh| format!("{kwh:.3}")).collect(),
                format!("{:.3}", point.export),
                point.ev_comparison.map(|comparison| format!("{:.3}", comparison.octopus)).unwrap_or_default(),
                source(point),
            )
        })
        .collect();
    let totals: Vec<String> = (0..electricity.labels.len())
        .map(|index| format!("{:.3}", electricity.points.iter().map(|point| point.import[index]).sum::<f64>()))
        .collect();
    let total_export = format!("{:.3}", electricity.points.iter().map(|point| point.export).sum::<f64>());

    rsx! {
        table { class: "telemetry-table",
            tr { class: "header",
                th { "Date" }
                th { "Start" }
                th { "End" }
                for label in electricity.labels.iter() {
                    th { class: "quantity", "{label}" br {} "(kWh)" }
                }
                th { class: "quantity", "{EXPORT_LABEL}" br {} "(kWh)" }
                th { class: "quantity", "{OCTOPUS_EV_LABEL}" br {} "(kWh)" }
                th { "Source" }
            }
            for (date, start, end, import, export, octopus_ev, source) in rows {
                tr {
                    td { "{date}" }
                    td { "{start}" }
                    td { "{end}" }
                    for kwh in import {
                        td { class: "numeric", "{kwh}" }
                    }
                    td { class: "numeric", "{export}" }
                    td { class: "numeric derived", "{octopus_ev}" }
                    td { "{source}" }
                }
            }
            tr { class: "total",
                td { "Total" }
                td {}
                td {}
                for total in totals {
                    td { class: "numeric derived", "{total}" }
                }
                td { class: "numeric derived", "{total_export}" }
                td { class: "numeric derived", {ev_comparison(&electricity.points).map(|comparison| format!("{:.3}", comparison.octopus)).unwrap_or_default()} }
                td {}
            }
        }
    }
}

/// A bar for each period, with the import of each label stacked above zero and the export below it.
#[allow(clippy::too_many_arguments)]
fn electricity_graph(electricity: &Electricity, start: &DateTime, end: &DateTime, period: TelemetryPeriod, mode: ElectricityMode,
    filter: &CategoryFilter, tz: &'static Tz, on_drill: impl FnMut((i64, i64)) + 'static, on_zoom: impl FnMut((i64, i64)) + 'static,
    menu: Vec<BarMenuItem>) -> Element
{
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let seconds_format = time::format_description::parse("[hour]:[minute]:[second]").unwrap();
    let day_format = time::format_description::parse("[day] [month repr:short]").unwrap();
    let month_format = time::format_description::parse("[month repr:short] [year]").unwrap();
    let long_month_format = time::format_description::parse("[month repr:long] [year]").unwrap();
    let local = |timestamp: i64| time::OffsetDateTime::from_unix_timestamp(timestamp).map(|time| time.to_timezone(tz)).ok();

    let several_days = local(start.unix_timestamp()).map(|time| time.date()) != local(end.unix_timestamp() - 1).map(|time| time.date());
    let x_bands = if !period.is_calendar() && several_days {
        day_bands(start.unix_timestamp(), end.unix_timestamp(), tz)
    }
    else {
        Vec::new()
    };

    // The value of each import label and of the export, as a negative, for each point, as the filter shows them.
    let points = &electricity.points;
    let values: Vec<(Vec<f64>, f64)> = points.iter().map(|point| mode.values(point, &electricity.labels)).collect();
    // Octopus's split of the car's import, dashed, and the measured one, solid, each half hourly, if they are shown, see
    // CategoryFilterMenu, which shows or hides both as Octopus's EV split.
    let line_style = filter.styles(&[String::from(OCTOPUS_EV_LABEL)]).remove(0);
    let compared = line_style.visible && points.iter().any(|point| point.ev_comparison.is_some());
    let mut categories: Vec<(String, Vec<f64>)> = electricity.labels.iter()
        .enumerate()
        .map(|(index, label)| (label.clone(), values.iter().map(|(import, _)| import[index]).collect()))
        .chain([(String::from(EXPORT_LABEL), values.iter().map(|(_, export)| -export).collect())])
        .collect();
    // With the lines, the car's import is at the bottom of the bars, so that the lines, which are drawn up from zero, are
    // level with its top.
    if compared {
        categories.sort_by_key(|(label, _)| !electricity::is_ev(label));
    }
    let shown: Vec<CategoryValues> = filter.apply(categories, points.len(), false);
    let line = |label: &str, value: fn(&EvComparison) -> f64, dashed: bool| StackedBarLine {
        label: String::from(label),
        color: line_style.color.clone(),
        values: points.iter()
            .map(|point| point.ev_comparison.as_ref().map(value).map(|kwh| match mode {
                ElectricityMode::Energy => kwh,
                ElectricityMode::Power => kwh / ((point.end - point.start).max(1) as f64 / 3600.0),
            }))
            .collect(),
        dashed,
    };
    let lines = if compared {
        vec![line(MEASURED_EV_LABEL, |comparison| comparison.measured, false), line(OCTOPUS_EV_LABEL, |comparison| comparison.octopus, true)]
    }
    else {
        Vec::new()
    };

    let bars: Vec<StackedBarPoint> = points.iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let (point_start, point_end) = (local(point.start)?, local(point.end)?);
            let ((tick_label, tick_rank), period_label) = match period {
                TelemetryPeriod::Day => (
                    (Some(point_start.format(&day_format).unwrap_or_default()), 0),
                    point_start.format(&date_format).unwrap_or_default(),
                ),
                TelemetryPeriod::Week => (
                    (Some(format!("Week of {}", point_start.format(&day_format).unwrap_or_default())), 0),
                    format!("Week of {}", point_start.format(&date_format).unwrap_or_default()),
                ),
                TelemetryPeriod::Month => (
                    (Some(point_start.format(&month_format).unwrap_or_default()), 0),
                    point_start.format(&long_month_format).unwrap_or_default(),
                ),
                _ => (
                    time_tick_label(point.start, point.end, tz),
                    format!(
                        "{} {}–{}",
                        point_start.format(&date_format).unwrap_or_default(),
                        point_start.format(&seconds_format).unwrap_or_default(),
                        point_end.format(&seconds_format).unwrap_or_default(),
                    ),
                ),
            };
            Some(StackedBarPoint {
                start: point.start,
                end: point.end,
                tick_label,
                tick_rank,
                period_label: format!("{period_label} ({})", source(point)),
                values: shown.iter().map(|category| category.values[index]).collect(),
            })
        })
        .collect();

    rsx! {
        StackedBarGraph {
            points: bars,
            series: shown.iter()
                .map(|category| StackedBarSeries { label: category.label.clone(), color: category.color.clone() })
                .collect::<Vec<_>>(),
            mode: StackedBarMode::Units,
            currency: Currency::NONE,
            unit: String::from(mode.unit()),
            // Energy is drawn as a histogram, as bars can have different lengths, e.g. a reading after a missing one,
            // half hours with only the meters' readings, or months, so that each bar's area is its energy.
            per_seconds: (mode == ElectricityMode::Energy).then(|| period.seconds().unwrap_or(86400)),
            on_drill: period.shorter().map(|_| EventHandler::new(on_drill)),
            on_zoom: (end.unix_timestamp() - start.unix_timestamp() > HALF_HOUR).then(|| EventHandler::new(on_zoom)),
            menu,
            x_bands,
            lines,
        }
    }
}
