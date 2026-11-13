//! The electricity imported and exported of any module, as a table or a graph with import above zero split into its
//! labels, e.g. the rates it is billed at, and export below zero, from what the module's ElectricitySource loads, see
//! power.rs. Octopus's has the car's import as Octopus split it as a line over the bars, to compare.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::anyhow;
use bruce_graphql::types::{Currency, DateTime};
use dioxus::prelude::*;
use dioxus_free_icons::{Icon, icons::ld_icons::LdRefreshCw};
use time_tz::{OffsetDateTimeExt, Tz};

use crate::components::app::Route;
use crate::components::category_filter::{CategoryFilter, CategoryFilterMenu, CategoryValues, STANDING_CHARGE_LABEL};
use crate::components::date_range_picker::{DateRangePicker, LocalSlot, RangeSelection};
use crate::components::error_text::ErrorText;
use crate::components::export::{ExportButton, ExportTable, ExportValue, export_file_name};
use crate::components::stacked_bar_graph::{
    BarClick, BarMenuItem, GraphMode, MAX_BARS, StackedBarGraph, StackedBarLine, StackedBarMode, StackedBarPoint,
    StackedBarSeries, day_bands, time_tick_label,
};
use crate::components::toggle_button::ToggleButton;
use crate::data_set::CancelToken;
use crate::period::Period;
use crate::power::measured::{MeasurementSource, Measurements};
use crate::power::topology::{CAR, TopologySplit};
use crate::power::{
    self, EXPORT_LABEL, Electricity, ElectricityPoint, ElectricitySource, EvComparison, LoadedElectricity,
    MEASURED_EV_LABEL, OCTOPUS_EV_LABEL,
};
use crate::profile::Settings as ProfileSettings;
use crate::util::SharedProp;
use crate::{AppContext, ModuleRegistrations};

/// The most rows in the table.
const MAX_SHOWN: usize = 9000;
const HALF_HOUR: i64 = 1800;

#[derive(Clone, Copy, PartialEq)]
enum ElectricityView {
    Table,
    Graph,
    /// Where the energy came from and where it went, see power::flows, where the site's topology places measurements.
    Flows,
}

impl std::fmt::Display for ElectricityView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElectricityView::Table => write!(f, "Table"),
            ElectricityView::Graph => write!(f, "Graph"),
            ElectricityView::Flows => write!(f, "Flows"),
        }
    }
}

/// The import of each label, whose uses are uses, see Electricity::uses, and the export of a point as shown, the energy itself in kWh, or the average power
/// in kW, the energy for Cost, whose values are the costs, see cost_categories. For power, a point with the Home Mini's
/// average power, which is steadier than its energy, see ElectricityPoint::demand, uses it for the house's labels: the
/// car charges at a constant power, so its labels are from their energy, and the house's share the rest of the Home
/// Mini's power in proportion to their energy.
fn mode_values(mode: GraphMode, point: &ElectricityPoint, uses: &[String]) -> (Vec<f64>, f64) {
    let hours = (point.end - point.start).max(1) as f64 / 3600.0;
    match (mode, point.demand) {
        (GraphMode::Energy | GraphMode::Cost, _) => (point.import.clone(), point.export),
        (GraphMode::Power, None) => (
            point.import.iter().map(|kwh| kwh / hours).collect(),
            point.export / hours,
        ),
        (GraphMode::Power, Some(demand)) => {
            let is_ev = |index: usize| uses[index] == CAR;
            let ev_kw: f64 = (0..uses.len())
                .filter(|index| is_ev(*index))
                .map(|index| point.import[index] / hours)
                .sum();
            let home_kwh: f64 = (0..uses.len())
                .filter(|index| !is_ev(*index))
                .map(|index| point.import[index])
                .sum();
            let home_kw = (demand.max(0.0) / 1000.0 - ev_kw).max(0.0);
            let import = (0..uses.len())
                .map(|index| match (is_ev(index), home_kwh > 0.0) {
                    (true, _) => point.import[index] / hours,
                    (false, true) => home_kw * point.import[index] / home_kwh,
                    (false, false) => 0.0,
                })
                .collect();
            (import, (-demand).max(0.0) / 1000.0)
        }
    }
}

/// The unit of the values of a mode, the currency's symbol for Cost.
fn mode_unit(mode: GraphMode) -> &'static str {
    match mode {
        GraphMode::Energy => "kWh",
        GraphMode::Power => "kW",
        GraphMode::Cost => "",
    }
}

/// The cost of each point by category, in the minor units of Electricity::currency, see ElectricityPoint::costs: the
/// standing charge first, as on Octopus's page of a meter's readings, then the import's labels and the export's credit,
/// negative, leaving out those which cost nothing, e.g. import the meter has no reading for yet.
fn cost_categories(electricity: &Electricity) -> Vec<(String, Vec<f64>)> {
    let points = &electricity.points;
    std::iter::once((
        String::from(STANDING_CHARGE_LABEL),
        points.iter().map(|point| point.standing_charge).collect::<Vec<f64>>(),
    ))
    .chain(
        electricity
            .labels
            .iter()
            .enumerate()
            .map(|(index, label)| (label.clone(), points.iter().map(|point| point.costs[index]).collect())),
    )
    .chain(std::iter::once((
        String::from(EXPORT_LABEL),
        points.iter().map(|point| point.export_cost).collect(),
    )))
    .filter(|(_, values)| values.iter().any(|value| *value != 0.0))
    .collect()
}

/// Where a point's energy is from, for its tooltip and the table, e.g. Home Mini and Tesla.
fn point_sources(point: &ElectricityPoint) -> String {
    let sources: Vec<&str> = point.sources.iter().map(String::as_str).collect();
    match sources.as_slice() {
        [] => String::new(),
        [one] => String::from(*one),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The car's import measured and as Octopus split it, in kWh, over the points' half hours which have both, see
/// ElectricityPoint::ev_comparison, None if none have.
pub fn ev_comparison(points: &[ElectricityPoint]) -> Option<EvComparison> {
    points
        .iter()
        .filter_map(|point| point.ev_comparison)
        .reduce(|total, more| total + more)
}

/// The range of half hours containing the times from start to end, as unix timestamps, as ranges are in half hours.
fn half_hour_range(start: i64, end: i64, tz: &'static Tz) -> Option<RangeSelection> {
    let start = start.div_euclid(HALF_HOUR) * HALF_HOUR;
    let end = (end + HALF_HOUR - 1).div_euclid(HALF_HOUR) * HALF_HOUR;
    RangeSelection::from_unix_timestamps(start, end.max(start + HALF_HOUR), tz)
}

/// What was loaded for the range shown: each source's electricity, and what measured its use.
struct Loaded {
    electricity: Vec<Arc<dyn LoadedElectricity>>,
    measurements: Vec<Arc<dyn Measurements>>,
}

/// The electricity of sources, put together, see power::electricity, the first of which, e.g. a supplier's, gives the
/// timezone, the periods and the like, split by use by measurements if there are any, placed by the site's topology,
/// see power::topology, under the heading title, with a range picker, the period, a table or a graph of energy or
/// power, a filter of the labels shown, Reload and Export.
#[component]
pub fn ElectricityExplorer(
    title: String,
    sources: Vec<SharedProp<dyn ElectricitySource>>,
    measurements: Vec<SharedProp<dyn MeasurementSource>>,
) -> Element {
    let settings = use_context::<Signal<ProfileSettings>>();
    let topology = settings.read().topology.clone();
    let source = sources[0].clone();
    let tz = source.0.timezone();
    let periods = source.0.periods();
    // The last 7 days, so that e.g. both Octopus's meters' readings and the Home Mini's more recent ones are shown.
    let mut range = use_signal(|| {
        let today = crate::clock::today(tz);
        RangeSelection {
            start: LocalSlot {
                date: today - time::Duration::days(6),
                slot: 0,
            },
            end: None,
        }
    });
    let mut view = use_signal(|| ElectricityView::Graph);
    // Whether a source's readings are shown alone, with nothing measured to split them, so that at its reading period
    // they are shown as it gave them, which is then the period first shown, e.g. the page of one GloBird register.
    let alone = sources.len() == 1 && measurements.is_empty();
    let reading_period = source.0.reading_period();
    let mut period = use_signal(|| match reading_period.filter(|_| alone) {
        Some(reading_period) => reading_period,
        None if periods.contains(&Period::HalfHour) => Period::HalfHour,
        None => periods[0],
    });
    let mut mode = use_signal(|| GraphMode::Energy);
    let filter = crate::components::category_filter::use_category_filter();
    // The range and period before each drill down or zoom in the graph, the latest last, for Zoom Out.
    let mut history = use_signal(Vec::<(RangeSelection, Period)>::new);
    // A period which is fetched again however up to date it is, set by Reload, see Telemetry.
    let mut reload = use_signal(|| None::<(DateTime, DateTime)>);
    let progress = use_signal(String::new);
    let cancel = use_hook(|| Rc::new(RefCell::new(CancelToken::new())));
    let mut cancelling = use_signal(|| false);

    let mut readings_resource = use_resource({
        let sources = sources.clone();
        let measurements = measurements.clone();
        let cancel = cancel.clone();
        move || {
            let sources = sources.clone();
            let measurements = measurements.clone();
            let dates = range().to_date_times(tz);
            let reload_period = *reload.peek();
            let token = CancelToken::new();
            *cancel.borrow_mut() = token.clone();
            async move {
                cancelling.set(false);
                let (start, end) = dates.map_err(|error| anyhow!(error))?;
                let mut electricity = Vec::new();
                for source in &sources {
                    let started = std::time::Instant::now();
                    electricity.push(
                        source
                            .0
                            .load(start, end, reload_period, progress, token.clone())
                            .await?,
                    );
                    crate::logging::log_if_slow(&format!("{}'s electricity", source.0.name()), started);
                }
                // The half hours either side too, as a split may need them.
                let mut loaded = Vec::new();
                for measurements in &measurements {
                    let name = measurements.0.name().to_string();
                    let mut progress = progress;
                    progress.set(format!("{name}: Loading what it measured"));
                    let started = std::time::Instant::now();
                    loaded.push(
                        measurements
                            .0
                            .load(start.unix_timestamp() - 1800, end.unix_timestamp() + 1800)
                            .await?,
                    );
                    crate::logging::log_if_slow(&format!("{name}'s measurements"), started);
                }
                let measurements = loaded;
                if reload_period.is_some() {
                    reload.set(None);
                }
                anyhow::Ok(Arc::new(Loaded {
                    electricity,
                    measurements,
                }))
            }
        }
    });
    let fetching = matches!(*readings_resource.state().read(), UseResourceState::Pending);
    let fetching_title = if reload.peek().is_some() {
        "Reloading"
    } else {
        "Loading"
    };

    let (start, end) = range().to_date_times(tz).map_err(|error| anyhow!(error))?;
    // As on the Home Mini page, periods which would be too many bars or rows can not be chosen, and the one shown is the
    // chosen one or the shortest longer one which fits.
    let most_shown = if view() == ElectricityView::Table {
        MAX_SHOWN
    } else {
        MAX_BARS
    };
    let fits = |option: Period| {
        option
            .seconds()
            .is_none_or(|seconds| ((end.unix_timestamp() - start.unix_timestamp()) / seconds) as usize <= most_shown)
    };
    let shown_period = periods
        .iter()
        .copied()
        .find(|option| *option >= period() && fits(*option))
        .unwrap_or(Period::Month);
    let readings: Option<Arc<Loaded>> = match &*readings_resource.read() {
        Some(Ok(readings)) => Some(readings.clone()),
        _ => None,
    };
    let electricity = readings.as_ref().map(|readings| {
        let loaded: Vec<&dyn LoadedElectricity> = readings.electricity.iter().map(|loaded| loaded.as_ref()).collect();
        let split = topology
            .clone()
            .filter(|_| !readings.measurements.is_empty())
            .map(|topology| TopologySplit {
                topology,
                measurements: readings.measurements.clone(),
            });
        power::electricity(
            &loaded,
            split.as_ref().map(|split| split as &dyn power::measured::Split),
            start.unix_timestamp(),
            end.unix_timestamp(),
            shown_period,
            tz,
        )
    });
    // Cost can be chosen if the meters' readings have costs, see ElectricityPoint::costs.
    let costed = electricity
        .as_ref()
        .is_some_and(|electricity| electricity.currency.is_some());
    let shown_mode = if mode() == GraphMode::Cost && !costed {
        GraphMode::Energy
    } else {
        mode()
    };
    let cost_values = electricity
        .as_ref()
        .filter(|_| shown_mode == GraphMode::Cost)
        .and_then(|electricity| Some((cost_categories(electricity), electricity.currency?)));
    // Where it came from and went, with the topology, of half hours at the shortest, see power::flows.
    let can_flow = topology.is_some()
        && readings
            .as_ref()
            .is_some_and(|readings| !readings.measurements.is_empty());
    // Of minutes at the shortest, as the zappi's records are.
    let flow_period = shown_period.max(Period::Minute);
    let flows = (view() == ElectricityView::Flows).then(|| {
        let (Some(topology), Some(readings)) = (topology.as_ref(), readings.as_ref()) else {
            return Vec::new();
        };
        let loaded: Vec<&dyn LoadedElectricity> = readings.electricity.iter().map(|loaded| loaded.as_ref()).collect();
        // The grid's readings of the period, or of half hours, which flows adds up into it.
        let detail = flow_period.min(Period::HalfHour);
        let electricity = power::electricity(&loaded, None, start.unix_timestamp(), end.unix_timestamp(), detail, tz);
        let grid: Vec<power::flows::GridReading> = electricity
            .points
            .iter()
            .map(|point| power::flows::GridReading {
                start: point.start,
                end: point.end,
                import: point.import.iter().sum(),
                export: point.export,
            })
            .collect();
        power::flows::flows(topology, &readings.measurements, &grid, flow_period, tz)
    });
    // The next shorter period to drill down to, if the source has it.
    let shorter = shown_period.shorter().filter(|shorter| periods.contains(shorter));
    let comparison = electricity
        .as_ref()
        .and_then(|electricity| ev_comparison(&electricity.points));
    // The import labels and export, if there is any, e.g. not on the page of one meter register, and Octopus's split
    // of the car's import if there is any to compare, for the categories menu.
    let categories: Vec<String> = match &flows {
        Some(flows) => {
            let (sources, uses) = flow_labels(flows);
            let mut all = sources;
            all.extend(uses.into_iter().filter(|label| label != power::flows::BATTERY));
            all
        }
        None if cost_values.is_some() => cost_values
            .iter()
            .flat_map(|(categories, _)| categories.iter().map(|(label, _)| label.clone()))
            .collect(),
        None => electricity
            .as_ref()
            .map(|electricity| {
                electricity
                    .labels
                    .iter()
                    .cloned()
                    .chain(has_export(electricity).then(|| String::from(EXPORT_LABEL)))
                    .chain(comparison.map(|_| String::from(OCTOPUS_EV_LABEL)))
                    .collect()
            })
            .unwrap_or_default(),
    };

    let on_drill = move |(bar_start, bar_end): (i64, i64)| {
        let Some(next) = shorter else {
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
                if let (Ok(bar_start), Ok(bar_end)) = (
                    DateTime::from_unix_timestamp(click.start),
                    DateTime::from_unix_timestamp(click.end),
                ) {
                    reload.set(Some((bar_start, bar_end)));
                    readings_resource.restart();
                }
            }),
        },
    ];

    // Whether the table's readings are worked out, e.g. added up into longer periods, rather than as the source gave
    // them, which is only known if it says what its readings' period is.
    let derived = reading_period.is_some_and(|reading_period| !alone || shown_period != reading_period);
    let export_electricity = electricity.clone();
    let compares_ev = source.0.compares_ev();
    let export_table = move |_: ()| {
        export_electricity
            .as_ref()
            .map(|electricity| points_export(electricity, compares_ev, tz))
    };
    let export_name = export_file_name("electricity", "all", &start, &end, &shown_period.to_string());

    let warnings: Vec<String> = readings
        .iter()
        .flat_map(|readings| readings.electricity.iter().filter_map(|loaded| loaded.warning()))
        .collect();
    let content = match (&*readings_resource.read(), &electricity) {
        (None, _) => rsx! {},
        (Some(Err(error)), _) => source.0.error(error),
        (Some(Ok(_)), Some(electricity)) if electricity.points.is_empty() => {
            rsx! { p { "No readings in this period" } }
        }
        (Some(Ok(_)), Some(electricity)) => rsx! {
            match view() {
                ElectricityView::Table => electricity_table(electricity, compares_ev, derived, tz),
                ElectricityView::Graph => electricity_graph(electricity, cost_values.clone(), &start, &end, shown_period, shorter.is_some(), shown_mode, &filter.read(), tz, on_drill, on_zoom, menu),
                ElectricityView::Flows => flows_graphs(flows.as_deref().unwrap_or_default(), &start, &end, flow_period, &filter.read(), tz,
                    shorter.is_some().then(|| EventHandler::new(on_drill)),
                    (end.unix_timestamp() - start.unix_timestamp() > HALF_HOUR).then(|| EventHandler::new(on_zoom))),
            }
            for loaded in readings.iter().flat_map(|readings| readings.electricity.iter()) {
                {loaded.notes(electricity)}
            }
            if topology.is_none() && !measurements.is_empty() {
                p {
                    "The import is not split into the car's, the batteries' and the house's until the site's topology is "
                    "set, on the Site page."
                }
            }
        },
        (Some(Ok(_)), None) => rsx! {},
    };

    // The site's time zone, which the times are in, see ElectricitySource::timezone, e.g. Sydney's for GloBird's.
    let zone = {
        use time_tz::TimeZone;
        format!(
            "Times in {} ({})",
            tz.name(),
            zone_name(crate::clock::now_utc().unix_timestamp(), tz)
        )
    };

    rsx! {
        h1 { "{title}" }
        p { class: "timezone", "{zone}" }
        for warning in warnings {
            p { class: "invalid", "{warning}" }
        }
        div { class: "date-range date-range-row",
            DateRangePicker { range, timezone: tz }
            span { class: "view-toggle",
                for option in [ElectricityView::Table, ElectricityView::Graph, ElectricityView::Flows].into_iter()
                    .filter(|option| *option != ElectricityView::Flows || can_flow)
                {
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
                    for option in GraphMode::ALL.into_iter().filter(|option| *option != GraphMode::Cost || costed) {
                        button {
                            r#type: "button",
                            class: if shown_mode == option { "active" },
                            onclick: move |_| mode.set(option),
                            "{option}"
                        }
                    }
                }
            }
            span { class: "view-toggle",
                for option in periods.iter().copied() {
                    ToggleButton {
                        label: option.to_string(),
                        active: shown_period == option,
                        unavailable: (!fits(option)).then(|| String::from("Zoom in to see more")),
                        onclick: move |_| period.set(option),
                    }
                }
            }
            if view() != ElectricityView::Table {
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
                title: source.0.reload_title(),
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

/// Whether any of the electricity was exported, as otherwise Export is left out of the graph.
fn has_export(electricity: &Electricity) -> bool {
    electricity.points.iter().any(|point| point.export != 0.0)
}

/// A message instead of more rows than can be shown.
fn too_many(count: usize) -> Element {
    rsx! {
        p { "That is {count} periods, too many to show. Choose a longer period or a shorter range." }
    }
}

/// The points as a table, for export.
fn points_export(electricity: &Electricity, compares_ev: bool, tz: &'static Tz) -> ExportTable {
    let mut columns = vec![String::from("Start"), String::from("End")];
    columns.extend(electricity.labels.iter().map(|label| format!("{label} (kWh)")));
    columns.push(format!("{EXPORT_LABEL} (kWh)"));
    if compares_ev {
        columns.push(format!("{OCTOPUS_EV_LABEL} (kWh)"));
    }
    columns.push(String::from("Source"));
    ExportTable {
        columns,
        rows: electricity
            .points
            .iter()
            .map(|point| {
                let mut row = vec![
                    ExportValue::unix_time(point.start, tz),
                    ExportValue::unix_time(point.end, tz),
                ];
                row.extend(point.import.iter().map(|kwh| ExportValue::number(*kwh)));
                row.push(ExportValue::number(point.export));
                if compares_ev {
                    row.push(
                        point
                            .ev_comparison
                            .map(|comparison| ExportValue::number(comparison.octopus))
                            .unwrap_or(ExportValue::Text(String::new())),
                    );
                }
                row.push(ExportValue::Text(point_sources(point)));
                row
            })
            .collect(),
    }
}

/// A row for each period, with the import of each label, the export and where they are from, and Octopus's split of
/// the car's import if compares_ev, see ElectricitySource::compares_ev, with the readings shown as derived if derived is
/// set, see ElectricityExplorer.
fn electricity_table(electricity: &Electricity, compares_ev: bool, derived: bool, tz: &'static Tz) -> Element {
    let reading_class = if derived { "numeric derived" } else { "numeric" };
    if electricity.points.len() > MAX_SHOWN {
        return too_many(electricity.points.len());
    }
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let time_format = time::format_description::parse("[hour]:[minute]:[second]").unwrap();
    let local = |timestamp: i64| {
        time::OffsetDateTime::from_unix_timestamp(timestamp)
            .map(|time| time.to_timezone(tz))
            .ok()
    };

    let mut previous_date = String::new();
    let rows: Vec<(String, String, String, Vec<String>, String, String, String)> = electricity
        .points
        .iter()
        .map(|point| {
            let (start, end) = (local(point.start), local(point.end));
            let date = start
                .map(|start| start.format(&date_format).unwrap_or_default())
                .unwrap_or_default();
            let shown_date = if date == previous_date {
                String::new()
            } else {
                date.clone()
            };
            previous_date = date;
            (
                shown_date,
                start
                    .map(|start| start.format(&time_format).unwrap_or_default())
                    .unwrap_or_default(),
                end.map(|end| end.format(&time_format).unwrap_or_default())
                    .unwrap_or_default(),
                point.import.iter().map(|kwh| format!("{kwh:.3}")).collect(),
                format!("{:.3}", point.export),
                point
                    .ev_comparison
                    .map(|comparison| format!("{:.3}", comparison.octopus))
                    .unwrap_or_default(),
                point_sources(point),
            )
        })
        .collect();
    let totals: Vec<String> = (0..electricity.labels.len())
        .map(|index| {
            format!(
                "{:.3}",
                electricity.points.iter().map(|point| point.import[index]).sum::<f64>()
            )
        })
        .collect();
    let total_export = format!(
        "{:.3}",
        electricity.points.iter().map(|point| point.export).sum::<f64>()
    );

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
                if compares_ev {
                    th { class: "quantity", "{OCTOPUS_EV_LABEL}" br {} "(kWh)" }
                }
                th { "Source" }
            }
            for (date, start, end, import, export, octopus_ev, source) in rows {
                tr {
                    td { "{date}" }
                    td { "{start}" }
                    td { "{end}" }
                    for kwh in import {
                        td { class: reading_class, "{kwh}" }
                    }
                    td { class: reading_class, "{export}" }
                    if compares_ev {
                        td { class: "numeric derived", "{octopus_ev}" }
                    }
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
                if compares_ev {
                    td { class: "numeric derived", {ev_comparison(&electricity.points).map(|comparison| format!("{:.3}", comparison.octopus)).unwrap_or_default()} }
                }
                td {}
            }
        }
    }
}

/// A bar for each period, with the import of each label stacked above zero and the export below it.
#[allow(clippy::too_many_arguments)]
/// The name of tz's zone at time, a unix timestamp, e.g. AEDT, or GMT, which changes when the clocks do, as the time
/// zone database gives it, which for some zones is the offset, e.g. +11.
fn zone_name(time: i64, tz: &'static Tz) -> String {
    use time_tz::{Offset, TimeZone};
    time::OffsetDateTime::from_unix_timestamp(time)
        .map(|time| tz.get_offset_utc(&time).name().to_string())
        .unwrap_or_default()
}

/// A bar's tick label, with how round its time is, see StackedBarPoint::tick_rank, and the label of its period, for its
/// tooltip, e.g. 2026-11-05 14:00:00–14:30:00, or Week of 2026-11-02, None if its times are out of range.
fn bar_labels(start: i64, end: i64, period: Period, tz: &'static Tz) -> Option<((Option<String>, u8), String)> {
    let date_format = time::macros::format_description!("[year]-[month]-[day]");
    let seconds_format = time::macros::format_description!("[hour]:[minute]:[second]");
    let day_format = time::macros::format_description!("[day] [month repr:short]");
    let month_format = time::macros::format_description!("[month repr:short] [year]");
    let long_month_format = time::macros::format_description!("[month repr:long] [year]");
    let local = |timestamp: i64| {
        time::OffsetDateTime::from_unix_timestamp(timestamp)
            .map(|time| time.to_timezone(tz))
            .ok()
    };
    let (point_start, point_end) = (local(start)?, local(end)?);
    Some(match period {
        Period::Day => (
            (Some(point_start.format(&day_format).unwrap_or_default()), 0),
            point_start.format(&date_format).unwrap_or_default(),
        ),
        Period::Week => (
            (
                Some(format!(
                    "Week of {}",
                    point_start.format(&day_format).unwrap_or_default()
                )),
                0,
            ),
            format!("Week of {}", point_start.format(&date_format).unwrap_or_default()),
        ),
        Period::Month => (
            (Some(point_start.format(&month_format).unwrap_or_default()), 0),
            point_start.format(&long_month_format).unwrap_or_default(),
        ),
        // With the zone, as the site's may not be the computer's, e.g. Sydney's.
        _ => (
            time_tick_label(start, end, tz),
            format!(
                "{} {}–{} {}",
                point_start.format(&date_format).unwrap_or_default(),
                point_start.format(&seconds_format).unwrap_or_default(),
                point_end.format(&seconds_format).unwrap_or_default(),
                zone_name(start, tz),
            ),
        ),
    })
}

/// The graph of electricity, or of its costs, by category, and their currency, if cost_values, see cost_categories.
fn electricity_graph(
    electricity: &Electricity,
    cost_values: Option<(Vec<(String, Vec<f64>)>, Currency)>,
    start: &DateTime,
    end: &DateTime,
    period: Period,
    drill: bool,
    mode: GraphMode,
    filter: &CategoryFilter,
    tz: &'static Tz,
    on_drill: impl FnMut((i64, i64)) + 'static,
    on_zoom: impl FnMut((i64, i64)) + 'static,
    menu: Vec<BarMenuItem>,
) -> Element {
    let local = |timestamp: i64| {
        time::OffsetDateTime::from_unix_timestamp(timestamp)
            .map(|time| time.to_timezone(tz))
            .ok()
    };

    let several_days = local(start.unix_timestamp()).map(|time| time.date())
        != local(end.unix_timestamp() - 1).map(|time| time.date());
    let x_bands = if !period.is_calendar() && several_days {
        day_bands(start.unix_timestamp(), end.unix_timestamp(), tz)
    } else {
        Vec::new()
    };

    // The value of each import label and of the export, as a negative, for each point, as the filter shows them.
    let points = &electricity.points;
    let values: Vec<(Vec<f64>, f64)> = points
        .iter()
        .map(|point| mode_values(mode, point, &electricity.uses))
        .collect();
    // Octopus's split of the car's import, dashed, and the measured one, solid, each half hourly, if they are shown, see
    // CategoryFilterMenu, which shows or hides both as Octopus's EV split.
    let line_style = filter.styles(&[String::from(OCTOPUS_EV_LABEL)]).remove(0);
    let compared =
        line_style.visible && cost_values.is_none() && points.iter().any(|point| point.ev_comparison.is_some());
    let currency = cost_values.as_ref().map_or(Currency::NONE, |(_, currency)| *currency);
    let mut categories: Vec<(String, Vec<f64>)> = match cost_values {
        Some((categories, _)) => categories,
        None => electricity
            .labels
            .iter()
            .enumerate()
            .map(|(index, label)| (label.clone(), values.iter().map(|(import, _)| import[index]).collect()))
            .chain(has_export(electricity).then(|| {
                (
                    String::from(EXPORT_LABEL),
                    values.iter().map(|(_, export)| -export).collect(),
                )
            }))
            .collect(),
    };
    // With the lines, the car's import is at the bottom of the bars, so that the lines, which are drawn up from zero, are
    // level with its top.
    if compared {
        let car: Vec<&String> = electricity
            .labels
            .iter()
            .zip(&electricity.uses)
            .filter(|(_, category)| *category == CAR)
            .map(|(label, _)| label)
            .collect();
        categories.sort_by_key(|(label, _)| !car.contains(&label));
    }
    let shown: Vec<CategoryValues> = filter.apply(categories, points.len(), false);
    let line = |label: &str, value: fn(&EvComparison) -> f64, dashed: bool| StackedBarLine {
        label: String::from(label),
        color: line_style.color.clone(),
        values: points
            .iter()
            .map(|point| {
                point.ev_comparison.as_ref().map(value).map(|kwh| match mode {
                    GraphMode::Energy | GraphMode::Cost => kwh,
                    GraphMode::Power => kwh / ((point.end - point.start).max(1) as f64 / 3600.0),
                })
            })
            .collect(),
        dashed,
    };
    let lines = if compared {
        vec![
            line(MEASURED_EV_LABEL, |comparison| comparison.measured, false),
            line(OCTOPUS_EV_LABEL, |comparison| comparison.octopus, true),
        ]
    } else {
        Vec::new()
    };

    let bars: Vec<StackedBarPoint> = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let ((tick_label, tick_rank), period_label) = bar_labels(point.start, point.end, period, tz)?;
            Some(StackedBarPoint {
                start: point.start,
                end: point.end,
                tick_label,
                tick_rank,
                period_label,
                detail: Some(format!("From {}", point_sources(point))).filter(|_| !point.sources.is_empty()),
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
            mode: mode.bar_mode(),
            currency,
            unit: String::from(mode_unit(mode)),
            // Energy and cost are drawn as a histogram, as bars can have different lengths, e.g. a reading after a
            // missing one, half hours with only the meters' readings, or months, so that each bar's area is its total.
            per_seconds: (mode != GraphMode::Power).then(|| period.seconds().unwrap_or(86400)),
            on_drill: drill.then(|| EventHandler::new(on_drill)),
            on_zoom: (end.unix_timestamp() - start.unix_timestamp() > HALF_HOUR).then(|| EventHandler::new(on_zoom)),
            menu,
            x_bands,
            lines,
        }
    }
}

/// What Home's Electricity and Site pages found in the profile's modules, see ElectricityPage.
pub(crate) struct Found {
    /// The sources, suppliers' first, see SourceKind.
    pub(crate) sources: Vec<Arc<dyn ElectricitySource>>,
    pub(crate) measurements: Vec<Arc<dyn MeasurementSource>>,
    /// The places they measure, see power::topology.
    pub(crate) channels: Vec<crate::power::topology::Channel>,
    /// The modules which are not logged in yet, which may have electricity to show.
    pub(crate) not_ready: Vec<String>,
    /// The modules, kept while their sources are used.
    _modules: Vec<Box<dyn crate::Module + Send>>,
}

/// Builds each of the profile's modules, as when its own pages are shown, see views/module.rs, for their electricity
/// measurements and channels.
pub(crate) async fn find(context: Arc<AppContext>, registrations: ModuleRegistrations) -> anyhow::Result<Found> {
    let mut found = Found {
        sources: Vec::new(),
        measurements: Vec::new(),
        channels: Vec::new(),
        not_ready: Vec::new(),
        _modules: Vec::new(),
    };
    for module_id in context.profile.active_profile.modules.keys() {
        let factory = registrations.module_factory(module_id, &context).await?;
        if !factory.is_ready().await? {
            found.not_ready.push(module_id.clone());
            continue;
        }
        let module = factory.build().await?;
        found.sources.extend(module.electricity_source());
        found.measurements.extend(module.measurement_source());
        found.channels.extend(module.channels());
        found._modules.push(module);
    }
    found.sources.sort_by_key(|source| source.kind());
    Ok(found)
}

/// The site's electricity, Home's first page, from the profile's modules: a supplier's, e.g. Octopus's, if there is
/// one, otherwise a device's which measures the grid, e.g. the Tesla gateway's, split by use by whatever measured it,
/// see power.rs. The modules do not know of each other: this puts them together.
#[component]
pub fn ElectricityPage() -> Element {
    let context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let registrations = use_context::<ModuleRegistrations>();
    let found = use_resource(move || {
        let context = context_signal.read().clone();
        let registrations = registrations.clone();
        async move { find(context.ok_or_else(|| anyhow!("There is no profile"))?, registrations).await }
    });
    let not_ready = |not_ready: &[String]| {
        rsx! {
            for module_id in not_ready.iter() {
                p {
                    "Log in to "
                    Link { to: Route::Module { module_id: module_id.clone() }, "{module_id}" }
                    " to see its electricity here."
                }
            }
        }
    };
    match &*found.read() {
        None => rsx! { p { "Loading..." } },
        Some(Err(error)) => {
            rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the electricity" } } }
        }
        Some(Ok(Found {
            sources,
            measurements,
            not_ready: waiting,
            ..
        })) if !sources.is_empty() => rsx! {
            ElectricityExplorer {
                title: "Electricity",
                sources: sources.iter().cloned().map(SharedProp).collect::<Vec<_>>(),
                measurements: measurements.iter().cloned().map(SharedProp).collect::<Vec<_>>(),
            }
            {not_ready(waiting)}
        },
        Some(Ok(Found { not_ready: waiting, .. })) => rsx! {
            h1 { "Electricity" }
            p { "None of this profile's modules has electricity to show, e.g. Octopus, GloBird or Tesla. Choose one on the Modules page." }
            {not_ready(waiting)}
        },
    }
}

/// The labels of flows' sources, Grid, Solar and Battery, and of their uses, the loads, e.g. EV and Home, then
/// Battery, Export and Not measured, of those any period has.
fn flow_labels(flows: &[power::flows::FlowPoint]) -> (Vec<String>, Vec<String>) {
    use power::flows::{BATTERY, GRID, NOT_MEASURED, SOLAR};
    let sources = [GRID, SOLAR, BATTERY]
        .into_iter()
        .filter(|label| flows.iter().any(|point| point.sources.contains_key(*label)))
        .map(String::from)
        .collect();
    let last = [BATTERY, EXPORT_LABEL, NOT_MEASURED];
    let mut uses: Vec<String> = flows
        .iter()
        .flat_map(|point| point.uses.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    // The car and the house first, as on the Graph, then the others by name, then the batteries, export and the rest.
    let (car, house) = (
        power::topology::label_prefix(CAR),
        power::topology::label_prefix(power::topology::HOUSE),
    );
    uses.sort_by_key(|label| {
        (
            label != car,
            label != house,
            last.iter().position(|last| last == label),
            label.clone(),
        )
    });
    (sources, uses)
}

/// Where the energy came from and where it went, see power::flows, as two graphs over the range, one above the other,
/// each in the categories' colours, as filter shows them, which on_drill and on_zoom show in more detail, as the Graph's
/// do.
fn flows_graphs(
    flows: &[power::flows::FlowPoint],
    start: &DateTime,
    end: &DateTime,
    period: Period,
    filter: &CategoryFilter,
    tz: &'static Tz,
    on_drill: Option<EventHandler<(i64, i64)>>,
    on_zoom: Option<EventHandler<(i64, i64)>>,
) -> Element {
    if flows.is_empty() {
        return rsx! { p { "No readings in this period" } };
    }
    let (sources, uses) = flow_labels(flows);
    let several_days = {
        let local = |timestamp: i64| {
            time::OffsetDateTime::from_unix_timestamp(timestamp)
                .map(|time| time.to_timezone(tz).date())
                .ok()
        };
        local(start.unix_timestamp()) != local(end.unix_timestamp() - 1)
    };
    let x_bands = if !period.is_calendar() && several_days {
        day_bands(start.unix_timestamp(), end.unix_timestamp(), tz)
    } else {
        Vec::new()
    };
    let graph = |labels: &[String], value: fn(&power::flows::FlowPoint) -> &std::collections::BTreeMap<String, f64>| {
        let categories: Vec<(String, Vec<f64>)> = labels
            .iter()
            .map(|label| {
                (
                    label.clone(),
                    flows
                        .iter()
                        .map(|point| value(point).get(label).copied().unwrap_or_default())
                        .collect(),
                )
            })
            .collect();
        let shown: Vec<CategoryValues> = filter.apply(categories, flows.len(), false);
        let bars: Vec<StackedBarPoint> = flows
            .iter()
            .enumerate()
            .filter_map(|(index, point)| {
                let ((tick_label, tick_rank), period_label) = bar_labels(point.start, point.end, period, tz)?;
                Some(StackedBarPoint {
                    start: point.start,
                    end: point.end,
                    tick_label,
                    tick_rank,
                    period_label,
                    detail: None,
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
                unit: String::from("kWh"),
                per_seconds: Some(period.seconds().unwrap_or(86400)),
                x_bands: x_bands.clone(),
                on_drill,
                on_zoom,
            }
        }
    };
    rsx! {
        h2 { "Where it came from" }
        {graph(&sources, |point| &point.sources)}
        h2 { "Where it went" }
        {graph(&uses, |point| &point.uses)}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_zone_is_named_as_the_clocks_are_then() {
        // 5 November 2026, summer in Sydney and winter in London, and early July.
        let november = 1_793_836_800;
        assert_eq!(
            super::zone_name(november, time_tz::timezones::db::australia::SYDNEY),
            "AEDT"
        );
        assert_eq!(
            super::zone_name(november, time_tz::timezones::db::europe::LONDON),
            "GMT"
        );
        assert_eq!(
            super::zone_name(november - 123 * 86400, time_tz::timezones::db::europe::LONDON),
            "BST"
        );
    }

    #[test]
    fn costs_are_the_standing_charge_first_then_the_labels_and_the_export_leaving_out_none() {
        use super::*;
        let point = |costs: Vec<f64>, export_cost: f64, standing_charge: f64| ElectricityPoint {
            start: 0,
            end: 1800,
            import: vec![1.0, 1.0, 1.0],
            export: 0.0,
            costs,
            export_cost,
            standing_charge,
            demand: None,
            sources: Vec::new(),
            ev_comparison: None,
        };
        let labels = ["Home Low Rate", "Home Standard Rate", "Home Low Rate (estimated)"]
            .map(String::from)
            .to_vec();
        let electricity = Electricity {
            labels,
            uses: vec![String::new(); 3],
            points: vec![
                point(vec![1.0, 0.0, 0.0], -0.5, 0.25),
                point(vec![0.0, 10.0, 0.0], 0.0, 0.25),
            ],
            currency: None,
        };
        assert_eq!(
            cost_categories(&electricity),
            vec![
                (String::from(STANDING_CHARGE_LABEL), vec![0.25, 0.25]),
                (String::from("Home Low Rate"), vec![1.0, 0.0]),
                (String::from("Home Standard Rate"), vec![0.0, 10.0]),
                (String::from(EXPORT_LABEL), vec![-0.5, 0.0]),
            ]
        );
    }
}
