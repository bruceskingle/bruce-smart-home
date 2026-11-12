//! The readings of a smart meter every 10 seconds, from a consumer access device such as an Octopus Home Mini, as a table
//! or graphs of the energy or demand in each period. Only what the server returns is shown, its costs are always
//! empty.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;
use crate::components::error_text::ErrorText;
use dioxus_free_icons::{Icon, icons::ld_icons::LdRefreshCw};
use bruce_graphql::types::{Currency, DateTime};
use time_tz::{OffsetDateTimeExt, Tz};

use crate::components::date_range_picker::{DateRangePicker, LocalSlot, RangeSelection};
use crate::components::export::{ExportButton, ExportFormat, export_file_name, save_table};
use crate::components::toggle_button::ToggleButton;
use crate::data_set::CancelToken;
use crate::components::category_filter::{CategoryFilter, CategoryFilterMenu, SOLID};
use crate::octopus::context::OctopusContext;
use crate::components::stacked_bar_graph::{BarClick, BarMenuItem, MAX_BARS, StackedBarGraph, StackedBarMode, StackedBarPoint, StackedBarSeries, day_bands, time_tick_label};
use crate::octopus::telemetry::{self, TelemetryReading};
use crate::power::grid::{self, GridPoint};
use crate::period::Period;
use crate::octopus::trend_graph::{TrendGraph, TrendLegendItem, TrendLine, TrendSmoothing, cumulative_by_year, moving_average, range_years,
    year_positions};
use super::measurements::graphql_error_descriptions;

/// The most bars or rows drawn, a day of 10 second readings, including the 25 hour day when the clocks go back. More
/// would make the page too slow, e.g. a week of 10 second readings.
const MAX_SHOWN: usize = 9000;
const HALF_HOUR: i64 = 1800;
const BAR_COLOR: &str = "#06b6d4";
const EXPORT_COLOR: &str = "#f59e0b";

#[derive(Clone, Copy, PartialEq)]
enum TelemetryView {
    Table,
    Graph,
    /// A line for each year, to compare years.
    Trend,
}

impl std::fmt::Display for TelemetryView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TelemetryView::Table => write!(f, "Table"),
            TelemetryView::Graph => write!(f, "Graph"),
            TelemetryView::Trend => write!(f, "Trend"),
        }
    }
}

/// What the graphs show.
#[derive(Clone, Copy, PartialEq)]
enum TelemetryMode {
    /// The energy imported, in kWh.
    Energy,
    /// The average power, in kW.
    Demand,
}

impl TelemetryMode {
    const ALL: [TelemetryMode; 2] = [TelemetryMode::Energy, TelemetryMode::Demand];

    /// The value of a period, in the units of the graph: kWh imported or average kW.
    fn value(&self, point: &GridPoint) -> f64 {
        match self {
            TelemetryMode::Energy => point.energy / 1000.0,
            TelemetryMode::Demand => point.demand.unwrap_or_default() / 1000.0,
        }
    }

    /// The imported and, as a negative value so that it is drawn below zero, exported value of a period, in the units
    /// of the graph. The average power is net, so it is one or the other.
    fn values(&self, point: &GridPoint) -> Vec<f64> {
        match self {
            TelemetryMode::Energy => vec![point.energy / 1000.0, -point.export / 1000.0],
            TelemetryMode::Demand => {
                let demand = point.demand.unwrap_or_default() / 1000.0;
                vec![demand.max(0.0), demand.min(0.0)]
            }
        }
    }

    fn graph_mode(&self) -> StackedBarMode {
        StackedBarMode::Units
    }

    fn unit(&self) -> &'static str {
        match self {
            TelemetryMode::Demand => "kW",
            _ => "kWh",
        }
    }
}

impl std::fmt::Display for TelemetryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TelemetryMode::Energy => write!(f, "Energy"),
            TelemetryMode::Demand => write!(f, "Power"),
        }
    }
}

/// The range of half hours containing the times from start to end, as unix timestamps, as ranges are in half hours.
fn half_hour_range(start: i64, end: i64, tz: &'static Tz) -> Option<RangeSelection> {
    let start = start.div_euclid(HALF_HOUR) * HALF_HOUR;
    let end = (end + HALF_HOUR - 1).div_euclid(HALF_HOUR) * HALF_HOUR;
    RangeSelection::from_unix_timestamps(start, end.max(start + HALF_HOUR), tz)
}

#[component]
/// The readings of the meter esme_id, which is its ESME (or GSME) ID, provided by the consumer access device cad_id.
pub fn Telemetry(account_id: String, esme_id: String, cad_id: String, billing_timezone: &'static Tz) -> Element {
    let context = use_context::<Arc<OctopusContext>>();
    let tz = billing_timezone;
    // Today, as there are a lot of readings and they are fetched an hour at a time.
    let mut range = use_signal(|| {
        let today = crate::clock::today(tz);
        RangeSelection { start: LocalSlot { date: today, slot: 0 }, end: None }
    });
    let mut view = use_signal(|| TelemetryView::Graph);
    let mut period = use_signal(|| Period::HalfHour);
    let mut mode = use_signal(|| TelemetryMode::Energy);
    let mut smoothing = use_signal(|| TrendSmoothing::None);
    // The colours of the years in the trend graph.
    let filter = crate::components::category_filter::use_category_filter();
    // The range and period before each drill down or zoom in the graph, the latest last, for Zoom Out.
    let mut history = use_signal(Vec::<(RangeSelection, Period)>::new);
    // A period whose days are fetched again however up to date they are, set by Reload. The fetch does not depend on it,
    // Reload restarts the fetch, so that it is only used once.
    let mut reload = use_signal(|| None::<(DateTime, DateTime)>);
    // The latest progress message from the fetch.
    let progress = use_signal(String::new);
    // The cancel token of the fetch in progress, each fetch has a new one so that cancelling one does not cancel the next.
    let cancel = use_hook(|| Rc::new(RefCell::new(CancelToken::new())));
    // Set when the Cancel button is clicked, until the next fetch starts.
    let mut cancelling = use_signal(|| false);

    let mut readings_resource = use_resource({
        let context = context.clone();
        let account_id = account_id.clone();
        let esme_id = esme_id.clone();
        let cancel = cancel.clone();
        move || {
            let context = context.clone();
            let account_id = account_id.clone();
            let esme_id = esme_id.clone();
            let dates = range().to_date_times(tz);
            let reload_period = *reload.peek();
            let token = CancelToken::new();
            *cancel.borrow_mut() = token.clone();
            async move {
                cancelling.set(false);
                let (start, end) = dates.map_err(|error| anyhow::anyhow!(error))?;
                let monitor = move |message: String| { let mut progress = progress; progress.set(message) };
                let readings = telemetry::fetch(&account_id, &esme_id, &start, &end, reload_period, tz, &context.cache_config,
                    &*context.request_manager, &monitor, &token).await;
                if reload_period.is_some() {
                    reload.set(None);
                }
                readings.map(Arc::new)
            }
        }
    });
    let fetching = matches!(*readings_resource.state().read(), UseResourceState::Pending);
    let fetching_title = if reload.peek().is_some() { "Reloading" } else { "Loading" };

    let (start, end) = range().to_date_times(tz).map_err(|error| anyhow::anyhow!(error))?;
    // The periods that would be too many bars or rows over the range can not be chosen, and the one shown is the
    // chosen one or, if that is too short, the shortest longer one that fits, so that zooming in shows the chosen one again.
    let most_shown = if view() == TelemetryView::Graph { MAX_BARS } else { MAX_SHOWN };
    let fits = |option: Period| option.seconds()
        .is_none_or(|seconds| ((end.unix_timestamp() - start.unix_timestamp()) / seconds) as usize <= most_shown);
    let shown_period = Period::ALL.into_iter()
        .find(|option| *option >= period() && fits(*option))
        .unwrap_or(Period::Month);
    let readings: Option<Arc<Vec<TelemetryReading>>> = match &*readings_resource.read() {
        Some(Ok(readings)) => Some(readings.clone()),
        _ => None,
    };
    let points = readings.as_ref()
        .map(|readings| grid::aggregate(readings, start.unix_timestamp(), end.unix_timestamp(), shown_period, tz))
        .unwrap_or_default();

    // Double clicking a bar shows it in more detail, at the next shorter period.
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
    // Dragging across bars shows just those bars, with the same period.
    let on_zoom = move |(zoom_start, zoom_end): (i64, i64)| {
        // Ranges are whole half hours, so a drag within the one half hour shown changes nothing.
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
            label: String::from("Export this bar as CSV"),
            enabled: readings.is_some(),
            action: EventHandler::new({
                let readings = readings.clone();
                let esme_id = esme_id.clone();
                move |click: BarClick| export_bar(&readings, &esme_id, click, ExportFormat::Csv, tz)
            }),
        },
        BarMenuItem {
            label: String::from("Export this bar as JSON"),
            enabled: readings.is_some(),
            action: EventHandler::new({
                let readings = readings.clone();
                let esme_id = esme_id.clone();
                move |click: BarClick| export_bar(&readings, &esme_id, click, ExportFormat::Json, tz)
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

    // Export what is shown, the periods at the chosen length.
    let export_points = points.clone();
    let export_table = move |_: ()| Some(telemetry::points_export(&export_points, tz));
    let export_name = export_file_name("telemetry", &esme_id, &start, &end, &shown_period.to_string());

    let content = match &*readings_resource.read() {
        // The loading dialog is over the page.
        None => rsx! {},
        Some(Err(error)) => match graphql_error_descriptions(error) {
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
        Some(Ok(readings)) if readings.is_empty() => rsx! {
            p { "No readings in this period, Octopus only has the last 30 days and older ones are only shown if they were saved." }
        },
        Some(Ok(_)) => match view() {
            TelemetryView::Table => readings_table(&points, tz),
            TelemetryView::Graph => readings_graph(&points, &start, &end, shown_period, mode(), tz, on_drill, on_zoom, menu),
            TelemetryView::Trend => match &readings {
                Some(readings) => trend(readings, &start, &end, mode(), smoothing(), &filter.read(), tz),
                None => rsx! {},
            },
        },
    };

    rsx! {
        h1 { "Home Mini: {cad_id}" }
        p { class: "note", "Readings every 10 seconds of the meter {esme_id}. Octopus only has the last 30 days, older readings are shown if they were saved." }
        div { class: "date-range date-range-row",
            DateRangePicker { range, timezone: tz }
            span { class: "view-toggle",
                for option in [TelemetryView::Table, TelemetryView::Graph, TelemetryView::Trend] {
                    button {
                        r#type: "button",
                        class: if view() == option { "active" },
                        onclick: move |_| view.set(option),
                        "{option}"
                    }
                }
            }
            if view() != TelemetryView::Table {
                span { class: "view-toggle",
                    for option in TelemetryMode::ALL {
                        button {
                            r#type: "button",
                            class: if mode() == option { "active" },
                            onclick: move |_| mode.set(option),
                            "{option}"
                        }
                    }
                }
            }
            // The trend is of the daily totals.
            if view() != TelemetryView::Trend {
                span { class: "view-toggle",
                    for option in Period::ALL {
                        ToggleButton {
                            label: option.to_string(),
                            active: shown_period == option,
                            unavailable: (!fits(option)).then(|| String::from("Zoom in to see more")),
                            onclick: move |_| period.set(option),
                        }
                    }
                }
            }
            if view() == TelemetryView::Trend {
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
                CategoryFilterMenu {
                    filter,
                    categories: Vec::<String>::new(),
                    years: range_years(&start, &end),
                    trend: true,
                    disabled: false,
                    visibility_only: true,
                }
            }
            button {
                r#type: "button",
                title: "Fetch the readings shown from Octopus again, as far as it still has them",
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
                disabled: readings.is_none(),
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

/// Saves the readings in the bar clicked.
fn export_bar(readings: &Option<Arc<Vec<TelemetryReading>>>, esme_id: &str, click: BarClick, format: ExportFormat, tz: &'static Tz) {
    let Some(readings) = readings else {
        return;
    };
    let in_bar: Vec<TelemetryReading> = readings.iter()
        .filter(|reading| reading.read_at >= click.start && reading.read_at < click.end)
        .cloned()
        .collect();
    let table = telemetry::readings_export(&in_bar, tz);
    let (Ok(start), Ok(end)) = (DateTime::from_unix_timestamp(click.start), DateTime::from_unix_timestamp(click.end)) else {
        return;
    };
    let name = export_file_name("telemetry", esme_id, &start, &end, "readings");
    spawn(async move {
        if let Err(error) = save_table(&table, &name, format).await {
            crate::log_error!("Export of a bar failed: {error}");
        }
    });
}

/// A message instead of more bars or rows than can be shown.
fn too_many(count: usize) -> Element {
    rsx! {
        p { "That is {count} periods, too many to show. Choose a longer period or a shorter range." }
    }
}

/// A row for each period with readings.
fn readings_table(points: &[GridPoint], tz: &'static Tz) -> Element {
    let rows: Vec<&GridPoint> = points.iter().filter(|point| point.readings > 0).collect();
    if rows.len() > MAX_SHOWN {
        return too_many(rows.len());
    }
    if rows.is_empty() {
        return rsx! { p { "No readings in this period" } };
    }
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let time_format = time::format_description::parse("[hour]:[minute]:[second]").unwrap();
    let local = |timestamp: i64| time::OffsetDateTime::from_unix_timestamp(timestamp).map(|time| time.to_timezone(tz)).ok();

    // The date is only shown when it is not the same as the row before.
    let mut previous_date = String::new();
    let cells: Vec<(String, String, String, String, String, String, usize)> = rows.iter()
        .map(|point| {
            let (start, end) = (local(point.start), local(point.end));
            let date = start.map(|start| start.format(&date_format).unwrap_or_default()).unwrap_or_default();
            let shown_date = if date == previous_date { String::new() } else { date.clone() };
            previous_date = date;
            (
                shown_date,
                start.map(|start| start.format(&time_format).unwrap_or_default()).unwrap_or_default(),
                end.map(|end| end.format(&time_format).unwrap_or_default()).unwrap_or_default(),
                format!("{:.3}", point.energy / 1000.0),
                format!("{:.3}", point.export / 1000.0),
                point.demand.map(|demand| format!("{:.3}", demand / 1000.0)).unwrap_or_default(),
                point.readings,
            )
        })
        .collect();

    let total_energy: f64 = rows.iter().map(|point| point.energy).sum();
    let total_export: f64 = rows.iter().map(|point| point.export).sum();
    let total_readings: usize = rows.iter().map(|point| point.readings).sum();
    // The average power over all the readings.
    let (demand_total, demand_readings) = rows.iter()
        .filter_map(|point| point.demand.map(|demand| (demand * point.readings as f64, point.readings)))
        .fold((0.0, 0), |(total, count), (demand, readings)| (total + demand, count + readings));
    let average_demand = if demand_readings > 0 { format!("{:.3}", demand_total / demand_readings as f64 / 1000.0) } else { String::new() };

    rsx! {
        table { class: "telemetry-table",
            tr { class: "header",
                th { "Date" }
                th { "Start" }
                th { "End" }
                th { class: "quantity", "Import" br {} "(kWh)" }
                th { class: "quantity", "Export" br {} "(kWh)" }
                th { class: "quantity", "Avg Power" br {} "(kW)" }
                th { class: "quantity", "Readings" }
            }
            for (date, start, end, energy, export, demand, readings) in cells {
                tr {
                    td { "{date}" }
                    td { "{start}" }
                    td { "{end}" }
                    td { class: "numeric", "{energy}" }
                    td { class: "numeric", "{export}" }
                    td { class: "numeric", "{demand}" }
                    td { class: "numeric", "{readings}" }
                }
            }
            tr { class: "total",
                td { "Total" }
                td {}
                td {}
                td { class: "numeric derived", {format!("{:.3}", total_energy / 1000.0)} }
                td { class: "numeric derived", {format!("{:.3}", total_export / 1000.0)} }
                td { class: "numeric derived", "{average_demand}" }
                td { class: "numeric derived", "{total_readings}" }
            }
        }
    }
}

/// A bar for each period, of the energy or average demand.
fn readings_graph(points: &[GridPoint], start: &DateTime, end: &DateTime, period: Period, mode: TelemetryMode, tz: &'static Tz,
    on_drill: impl FnMut((i64, i64)) + 'static, on_zoom: impl FnMut((i64, i64)) + 'static, menu: Vec<BarMenuItem>) -> Element
{
    if points.len() > MAX_SHOWN {
        return too_many(points.len());
    }
    let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let seconds_format = time::format_description::parse("[hour]:[minute]:[second]").unwrap();
    let day_format = time::format_description::parse("[day] [month repr:short]").unwrap();
    let month_format = time::format_description::parse("[month repr:short] [year]").unwrap();
    let long_month_format = time::format_description::parse("[month repr:long] [year]").unwrap();
    let local = |timestamp: i64| time::OffsetDateTime::from_unix_timestamp(timestamp).map(|time| time.to_timezone(tz)).ok();

    // Periods shorter than a day are labelled every 3 hours over one day, and have a band for each day over more.
    let several_days = local(start.unix_timestamp()).map(|time| time.date()) != local(end.unix_timestamp() - 1).map(|time| time.date());
    let x_bands = if !period.is_calendar() && several_days {
        day_bands(start.unix_timestamp(), end.unix_timestamp(), tz)
    }
    else {
        Vec::new()
    };

    let bars: Vec<StackedBarPoint> = points.iter()
        .filter_map(|point| {
            let (point_start, point_end) = (local(point.start)?, local(point.end)?);
            let ((tick_label, tick_rank), period_label) = match period {
                Period::Day => (
                    (Some(point_start.format(&day_format).unwrap_or_default()), 0),
                    point_start.format(&date_format).unwrap_or_default(),
                ),
                Period::Week => (
                    (Some(format!("Week of {}", point_start.format(&day_format).unwrap_or_default())), 0),
                    format!("Week of {}", point_start.format(&date_format).unwrap_or_default()),
                ),
                Period::Month => (
                    (Some(point_start.format(&month_format).unwrap_or_default()), 0),
                    point_start.format(&long_month_format).unwrap_or_default(),
                ),
                _ => (
                    // The roundest time in the bar, which at 10 seconds may not be its start.
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
                detail: None,
                start: point.start,
                end: point.end,
                tick_label,
                tick_rank,
                period_label,
                values: mode.values(point),
            })
        })
        .collect();

    rsx! {
        StackedBarGraph {
            points: bars,
            series: vec![
                StackedBarSeries { label: String::from("Import"), color: String::from(BAR_COLOR) },
                StackedBarSeries { label: String::from("Export"), color: String::from(EXPORT_COLOR) },
            ],
            mode: mode.graph_mode(),
            currency: Currency::NONE,
            unit: String::from(mode.unit()),
            // Energy is drawn as a histogram, as a reading after a missing one is longer, and months are, so that each
            // bar's area is its energy.
            per_seconds: (mode == TelemetryMode::Energy).then(|| period.seconds().unwrap_or(86400)),
            on_drill: period.shorter().map(|_| EventHandler::new(on_drill)),
            // A half hour is the shortest range, so one can not be zoomed into.
            on_zoom: (end.unix_timestamp() - start.unix_timestamp() > HALF_HOUR).then(|| EventHandler::new(on_zoom)),
            menu,
            x_bands,
        }
    }
}

/// A line graph over one year of the daily energy or average demand, with a line for each year from start to end.
fn trend(readings: &[TelemetryReading], start: &DateTime, end: &DateTime, mode: TelemetryMode, smoothing: TrendSmoothing,
    filter: &CategoryFilter, tz: &'static Tz) -> Element
{
    let points = grid::aggregate(readings, start.unix_timestamp(), end.unix_timestamp(), Period::Day, tz);
    let positions = year_positions(points.iter().map(|point| (point.start, point.end)), tz)?;
    let point_years: Vec<i32> = positions.iter().map(|(year, _)| *year).collect();
    let times: Vec<i64> = points.iter().map(|point| point.start).collect();
    let daily: Vec<f64> = points.iter().map(|point| mode.value(point)).collect();
    let values = match smoothing {
        TrendSmoothing::None => daily,
        TrendSmoothing::MovingAverage(days) => moving_average(&times, &daily, days),
        TrendSmoothing::Cumulative => cumulative_by_year(&point_years, &daily),
    };
    let suffix = if matches!(smoothing, TrendSmoothing::MovingAverage(_)) && mode != TelemetryMode::Demand { "/day" } else { "" };

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
            mode: mode.graph_mode(),
            currency: Currency::NONE,
            unit: String::from(mode.unit()),
            suffix: String::from(suffix),
        }
    }
}
