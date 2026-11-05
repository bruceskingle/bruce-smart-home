use std::sync::Arc;

use anyhow::anyhow;
use dioxus::prelude::*;
use time_tz::Tz;
use sparko_graphql::types::DateTime;

use crate::components::date_range_picker::{DateRangePicker, RangeSelection};
use crate::components::export::{ExportButton, ExportFormat, export_file_name, save_table};
use crate::components::toggle_button::ToggleButton;
use crate::data_set::CancelToken;
use crate::components::category_filter::{CategoryFilter, CategoryFilterMenu};
use crate::octopus::trend_graph::{TrendSmoothing, range_years};
use crate::octopus::context::OctopusContext;
use crate::octopus::measurements::MeasurementsDataSet;
// use crate::octopus::measurements::MeasurementsDevice;
use crate::octopus::meter_point::MeterPointDataSet;
use crate::octopus::smart_device::SmartDeviceDataSet;
use crate::octopus::property::{PropertyDataSet, smart_device_network_table};
use super::telemetry::Telemetry;
use crate::components::stacked_bar_graph::{BarClick, BarMenuItem, MAX_BARS, StackedBarMode, StackedBarPeriod};

/// Each GraphQL error from the server in error, as its message, its description, which is often advice about the
/// message, and its code, e.g. "Unable to find matching device for this request. Ensure the device id is correct.
/// (KT-CT-4716)", or None if it is not from GraphQL errors.
pub(super) fn graphql_error_descriptions(error: &anyhow::Error) -> Option<Vec<String>> {
    let errors = error.chain().find_map(|cause| match cause.downcast_ref::<sparko_graphql::Error>() {
        Some(sparko_graphql::Error::GraphQLError(errors)) => Some(errors.clone()),
        _ => None,
    })?;
    let text = |value: Option<&serde_json::Value>| value.and_then(|value| value.as_str()).map(String::from);
    Some(errors.iter()
        .map(|error| {
            let parts: Vec<String> = [error.message.clone(), text(error.extensions.get("errorDescription"))]
                .into_iter()
                .flatten()
                .collect();
            let description = if parts.is_empty() { String::from("Unknown error") } else { parts.join(" ") };
            match text(error.extensions.get("errorCode")) {
                Some(code) => format!("{description} ({code})"),
                None => description,
            }
        })
        .collect())
}

#[derive(Props, Clone, PartialEq)]
pub struct MeasurementsPageProps {
    path: Vec<String>,
}

#[component]
pub fn MeasurementsPage(props: MeasurementsPageProps) -> Element {
    let context = use_context::<Arc<OctopusContext>>();

    // Fetch the list of all meters (properties), this runs once when the component is mounted.
    let meter_points_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move { MeterPointDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await }
        }
    });

    // The properties, for the devices on their smart meter networks.
    let properties_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move { PropertyDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await }
        }
    });

    // The Smart Flex devices on the account.
    let devices_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move { SmartDeviceDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await }
        }
    });

    match &*meter_points_resource.read() {
        None => rsx! {
            div { "Loading Meter Points for account {context.account_id}..." }
        },
        Some(Err(error)) => Err(anyhow!("Failed to load Meter Points: {error:#}"))?,
        Some(Ok(meter_point_list)) => {

            // Are we looking at a device? Its path is device, its ID, modelled or measured, and the meter point to
            // fetch them for.
            // if let (Some("device"), Some(device_id), Some(mpan)) = (props.path.first().map(String::as_str), props.path.get(1), props.path.get(3)) {
                // let modelled = props.path.get(2).is_some_and(|mode| mode == "modelled");
                // let export = meter_point_list.meter_points.iter()
                    // .find(|meter_point| &meter_point.mpan_ == mpan)
                    // .is_some_and(|meter_point| matches!(meter_point.direction_, Some(crate::octopus::graphql::ElectricityDirection::Export)));
                // rsx! {
                    // MeasurementsConsumption {
                        // key: "{device_id}#{modelled}#{mpan}",
                        // account_id: context.account_id.clone(),
                        // mpan: mpan.clone(),
                        // export,
                        // device: MeasurementsDevice { id: device_id.clone(), modelled },
                        // billing_timezone: context.billing_timezone,
                    // }
                // }
            // }
            // else
            // Are we looking at a meter's readings every 10 seconds? Its path is telemetry, the meter's ESME ID and the ID
            // of the consumer access device which provides them.
            if let (Some("telemetry"), Some(esme_id)) = (props.path.first().map(String::as_str), props.path.get(1)) {
                let cad_id = props.path.get(2).cloned().unwrap_or_default();
                rsx! {
                    Telemetry {
                        key: "{esme_id}",
                        account_id: context.account_id.clone(),
                        esme_id: esme_id.clone(),
                        cad_id,
                        billing_timezone: context.billing_timezone,
                    }
                }
            }
            // Are we looking at one meter?
            else if let Some(mpan) = props.path.first() {
                    // Export meter points have generation readings rather than consumption.
                    let export = meter_point_list.meter_points.iter()
                        .find(|meter_point| &meter_point.mpan_ == mpan)
                        .is_some_and(|meter_point| matches!(meter_point.direction_, Some(crate::octopus::graphql::ElectricityDirection::Export)));
                    rsx! {
                        MeasurementsConsumption {
                            key: "{mpan}",
                            account_id: context.account_id.clone(),
                            mpan: mpan.clone(),
                            export,
                            // device: None,
                            billing_timezone: context.billing_timezone,
                        }
                    }
            }
            else {

                let mut lines = Vec::new();


                for meter_point in &meter_point_list.meter_points {

                    let product_code = meter_point.agreements_.first()
                        .and_then(|agreement| agreement.tariff_.as_ref())
                        .and_then(|tariff| match tariff {
                            crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType::StandardTariff(t) => t.product_code_.as_deref(),
                            crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType::DayNightTariff(t) => t.product_code_.as_deref(),
                            crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType::ThreeRateTariff(t) => t.product_code_.as_deref(),
                            crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType::FourRateEvTariff(t) => t.product_code_.as_deref(),
                            crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType::HalfHourlyTariff(t) => t.product_code_.as_deref(),
                            crate::octopus::graphql::meter::active_meter_points_with_agreements::ElectricityTariffType::PrepayTariff(t) => t.product_code_.as_deref(),

                        })
                        .unwrap_or("UNKNOWN");


                        
                    let direction = meter_point.direction_.as_ref().map_or("UNKNOWN".to_string(), |v| v.to_string());
                    let id = meter_point.mpan_.clone();
                    // Import and export meter points both have measurements, the direction is needed to fetch them.
                    let mpan_link = if meter_point.direction_.is_some() {
                        rsx! {
                            div {
                                class: "link",
                                onclick: move |_| {

                                    let mut path_signal = use_context::<Signal<Vec<String>>>();
                                    let new_path = vec![String::from("measurements"), id.clone()];
                                    path_signal.set(new_path);
                                },
                                {meter_point.mpan_.clone()}
                            }
                        }
                    }
                    else {
                        rsx! {
                            {meter_point.mpan_.clone()}
                        }
                    };
                    lines.push(rsx!{
                        tr {
                            td { {mpan_link} }
                            td { {direction} }
                            td { {product_code} }
                        }
                    })
                }


                // // The meter points a device's readings can be fetched for, export first, with their direction.
                // let mut device_meter_points: Vec<(String, String)> = meter_point_list.meter_points.iter()
                    // .filter_map(|meter_point| meter_point.direction_.as_ref().map(|direction| {
                        // let direction = if matches!(direction, crate::octopus::graphql::ElectricityDirection::Export) { "Export" } else { "Import" };
                        // (String::from(direction), meter_point.mpan_.clone())
                    // }))
                    // .collect();
                // device_meter_points.sort_by_key(|(direction, _)| direction != "Export");

                rsx! {
                    h1 { "Meter Points" }
                    table {
                        tr { class: "header",
                            th { "MPAN" }
                            th { "Direction" }
                            th { "Product Code" }
                        }

                        for line in lines {
                            {line}
                        }
                    }
                    // On all the account's properties, as the meter points are.
                    h1 { "Smart Device Network" }
                    match &*properties_resource.read() {
                        None => rsx! {
                            div { "Loading the smart device network..." }
                        },
                        Some(Err(error)) => rsx! {
                            div { class: "invalid", "Failed to load the smart device network: {error}" }
                        },
                        Some(Ok(properties)) => {
                            let properties: Vec<&crate::octopus::graphql::meter::account_properties_meters::PropertyType> = properties.property_map.values()
                                .map(|property| property.as_ref())
                                .collect();
                            smart_device_network_table(&properties)
                        },
                    }
                    h1 { "Smart Flex Devices" }
                    // The devices' readings were tried, with the measurements query filtered by deviceId and the meter
                    // point's MPAN, measured and with modelledSubMeterReadings (Measured and Modelled). On an account
                    // with an Octopus Home Mini and a myenergi zappi, Measured was refused (KT-CT-4717) and Modelled gave
                    // the meter's own readings filtered to the half hours the device was in use, so nothing new, and
                    // they are not shown. The commented out code here and in measurements.rs fetches them, which may be
                    // useful when other types of device are seen, e.g. batteries or heat pumps. A charge point's
                    // charging sessions are on the EV Charging page.
                    match &*devices_resource.read() {
                        None => rsx! {
                            div { "Loading Smart Flex devices..." }
                        },
                        Some(Err(error)) => rsx! {
                            div { class: "invalid", "Failed to load Smart Flex devices: {error}" }
                        },
                        Some(Ok(device_list)) if device_list.devices.is_empty() => rsx! {
                            div { "There are no Smart Flex devices on this account" }
                        },
                        Some(Ok(device_list)) => rsx! {
                            table {
                                tr { class: "header",
                                    th { "Name" }
                                    th { "Type" }
                                    th { "Provider" }
                                    // th { "Readings" }
                                }
                                for device in device_list.devices.iter().cloned() {
                                    tr {
                                        td { {device.name.clone().unwrap_or_default()} }
                                        td {
                                            // A charge point or vehicle links to its charging sessions on the EV Charging page.
                                            if device.charging.is_some() {
                                                span {
                                                    class: "link",
                                                    title: "Show its charging sessions",
                                                    onclick: {
                                                        let id = device.id.clone();
                                                        move |_| {
                                                            let mut path_signal = use_context::<Signal<Vec<String>>>();
                                                            path_signal.set(vec![String::from("ev"), id.clone()]);
                                                        }
                                                    },
                                                    "{device.device_type}"
                                                }
                                            }
                                            else {
                                                "{device.device_type}"
                                            }
                                        }
                                        td { "{device.provider}" }
                                        // td {
                                            // // Measured readings, and the readings the server models for the device, for
                                            // // each meter point, export first.
                                            // for (direction, mpan) in device_meter_points.iter().cloned() {
                                                // div {
                                                    // "{direction}: "
                                                    // for (label, mode) in [("Measured", "measured"), ("Modelled", "modelled")] {
                                                        // span {
                                                            // class: "link device-link",
                                                            // onclick: {
                                                                // let id = device.id.clone();
                                                                // let mpan = mpan.clone();
                                                                // move |_| {
                                                                    // let mut path_signal = use_context::<Signal<Vec<String>>>();
                                                                    // path_signal.set(vec![
                                                                        // String::from("measurements"),
                                                                        // String::from("device"),
                                                                        // id.clone(),
                                                                        // String::from(mode),
                                                                        // mpan.clone(),
                                                                    // ]);
                                                                // }
                                                            // },
                                                            // "{label}"
                                                        // }
                                                    // }
                                                // }
                                            // }
                                        // }
                                    }
                                }
                            }
                        },
                    }
                }
            }
        },
    }
}

#[derive(Props, Clone)]
struct MeasurementsConsumptionProps {
    account_id: String,
    mpan: String,
    /// Whether this is an export meter point.
    export: bool,
    // /// The device whose readings are shown for the meter point, instead of the meter point's own.
    // device: Option<MeasurementsDevice>,
    billing_timezone: &'static Tz,
}

impl PartialEq for MeasurementsConsumptionProps {
    fn eq(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.mpan == other.mpan
            && self.export == other.export
            // && self.device == other.device
            && std::ptr::eq(self.billing_timezone, other.billing_timezone)
    }
}

/*
 * Date range and view selection for a meter, the selected range is passed as props to MeterDetail
 * which does the fetch and display.
 */
#[component]
fn MeasurementsConsumption(props: MeasurementsConsumptionProps) -> Element {
    let tz = props.billing_timezone;

    // The range currently being displayed.
    // Default to the start of the current month with no end.
    let mut applied_range = use_signal(|| RangeSelection::current_month(tz));

    // Whether consumption is shown as a table or a graph, kept here so it survives range changes.
    let mut view = use_signal(|| ConsumptionView::Graph);
    let mut graph_mode = use_signal(|| StackedBarMode::Cost);
    let mut graph_period = use_signal(|| StackedBarPeriod::Raw);
    let mut smoothing = use_signal(|| TrendSmoothing::None);

    // The colour and visibility of each category in the graphs, kept here so it survives range changes.
    let category_filter = crate::components::category_filter::use_category_filter();

    // The data loaded by MeasurementsDetail for the applied range, None until it has loaded.
    let exportable = use_signal(|| None::<Arc<MeasurementsDataSet>>);

    // The range and period before each drill down or zoom in the graph, the latest last, for Zoom Out.
    let mut history = use_signal(Vec::<(RangeSelection, StackedBarPeriod)>::new);

    let selection = *applied_range.read();
    let (start, end) = selection.to_date_times(tz).map_err(|error| anyhow!(error))?;

    // The periods that would be too many bars in the graph over the range can not be chosen, and the one shown is the
    // chosen one or, if that is too long, the shortest longer one that fits, so that zooming in shows the chosen one
    // again. Rate periods depend on the tariff, so they are counted once the data is loaded.
    let fits = |option: StackedBarPeriod| {
        let seconds = end.unix_timestamp() - start.unix_timestamp();
        view() != ConsumptionView::Graph || match option {
            StackedBarPeriod::Raw => (seconds / 1800) as usize <= MAX_BARS,
            StackedBarPeriod::Rate => exportable.read().as_ref()
                .is_none_or(|data| data.graph_bars(&start, &end, tz, StackedBarPeriod::Rate) <= MAX_BARS),
            StackedBarPeriod::Day => (seconds / 86400) as usize <= MAX_BARS,
            StackedBarPeriod::Week | StackedBarPeriod::Month => true,
        }
    };
    let shown_period = [StackedBarPeriod::Raw, StackedBarPeriod::Rate, StackedBarPeriod::Day, StackedBarPeriod::Week]
        .into_iter()
        .find(|option| *option >= graph_period() && fits(*option))
        .unwrap_or(StackedBarPeriod::Month);

    // Double clicking a bar shows it in more detail: a month by week, a week by day, and a day as its raw readings. A
    // rate period, if Rate was chosen, shows the raw readings for its whole day (the period alone would look the same
    // as the bar).
    let on_drill = move |(bar_start, bar_end): (i64, i64)| {
        let (period, range) = match shown_period {
            StackedBarPeriod::Month => (StackedBarPeriod::Week, RangeSelection::from_unix_timestamps(bar_start, bar_end, tz)),
            StackedBarPeriod::Week => (StackedBarPeriod::Day, RangeSelection::from_unix_timestamps(bar_start, bar_end, tz)),
            StackedBarPeriod::Day => (StackedBarPeriod::Raw, RangeSelection::from_unix_timestamps(bar_start, bar_end, tz)),
            StackedBarPeriod::Rate => (StackedBarPeriod::Raw, RangeSelection::day_containing(bar_start, tz)),
            StackedBarPeriod::Raw => return,
        };
        if let Some(range) = range {
            history.write().push((applied_range(), shown_period));
            applied_range.set(range);
            graph_period.set(period);
        }
    };

    // Dragging across bars shows just those bars, with the same period.
    let on_zoom = move |(zoom_start, zoom_end): (i64, i64)| {
        if let Some(range) = RangeSelection::from_unix_timestamps(zoom_start, zoom_end, tz) {
            history.write().push((applied_range(), shown_period));
            applied_range.set(range);
        }
    };

    // The items in the menu shown when a bar is right clicked which change the range, the rest are added by
    // MeasurementsDetail as they need the data.
    let (previous_range, next_range) = applied_range().before_and_after(tz);
    let range_menu = vec![
        BarMenuItem {
            label: String::from("Zoom Out"),
            enabled: !history.read().is_empty(),
            action: EventHandler::new(move |_: BarClick| {
                if let Some((range, period)) = history.write().pop() {
                    applied_range.set(range);
                    graph_period.set(period);
                }
            }),
        },
        BarMenuItem {
            label: String::from("Previous"),
            enabled: previous_range.is_some(),
            action: EventHandler::new(move |_: BarClick| {
                if let Some(range) = previous_range {
                    applied_range.set(range);
                }
            }),
        },
        BarMenuItem {
            label: String::from("Next"),
            enabled: next_range.is_some(),
            action: EventHandler::new(move |_: BarClick| {
                if let Some(range) = next_range {
                    applied_range.set(range);
                }
            }),
        },
    ];

    // Export what is currently shown, the readings in the table or the aggregated graph values.
    let export_table = move |_: ()| {
        let data = exportable.read().clone()?;
        Some(match view() {
            ConsumptionView::Table => data.readings_export(&start, &end),
            ConsumptionView::Graph | ConsumptionView::Trend => data.graph_export(&start, &end, tz, shown_period),
        })
    };

    // The categories in the loaded data, for the category menu.
    let categories = exportable.read().as_ref()
        .map(|data| data.categories(&start, &end, graph_mode()))
        .unwrap_or_default();
    let export_name = export_file_name("measurements", &props.mpan, &start, &end, &view().export_description(shown_period));

    // FIXME: remove this hardcoded test date range
    // let start = DateTime::from_calendar_date_time(2026, time::Month::October, 1, 5, 0, 0)?;
    // let end = Some(DateTime::from_calendar_date_time(2026, time::Month::October, 1, 6, 0, 0)?);



    // The key causes MeterDetail to be re-mounted when the applied range changes, which drops
    // (and so cancels) any fetch in progress for the previous range and starts a new one.
    let detail = rsx! {
        MeasurementsDetail {
            key: "{start}#{end}",
            account_id: props.account_id.clone(),
            mpan: props.mpan.clone(),
            export: props.export,
            // device: props.device.clone(),
            billing_timezone: tz,
            start,
            end,
            view: view(),
            graph_mode: graph_mode(),
            graph_period: shown_period,
            smoothing: smoothing(),
            category_filter: category_filter(),
            exportable,
            on_drill: if shown_period == StackedBarPeriod::Raw { None } else { Some(EventHandler::new(on_drill)) },
            on_zoom: EventHandler::new(on_zoom),
            range_menu,
        }
    };

    rsx! {
        div { class: "date-range date-range-row",
            DateRangePicker { range: applied_range, timezone: tz }
            span { class: "view-toggle",
                for option in [ConsumptionView::Table, ConsumptionView::Graph, ConsumptionView::Trend] {
                    button {
                        r#type: "button",
                        class: if view() == option { "active" },
                        onclick: move |_| view.set(option),
                        "{option}"
                    }
                }
            }
            if view() != ConsumptionView::Table {
                span { class: "view-toggle",
                    for option in [StackedBarMode::Cost, StackedBarMode::Units] {
                        button {
                            r#type: "button",
                            class: if graph_mode() == option { "active" },
                            onclick: move |_| graph_mode.set(option),
                            "{option}"
                        }
                    }
                }
                span { class: "view-toggle",
                    for option in [
                        StackedBarPeriod::Raw,
                        StackedBarPeriod::Rate,
                        StackedBarPeriod::Day,
                        StackedBarPeriod::Week,
                        StackedBarPeriod::Month,
                    ]
                    {
                        ToggleButton {
                            label: option.to_string(),
                            active: shown_period == option,
                            // Smoothing works from daily totals, so the period is not used.
                            disabled: view() == ConsumptionView::Trend && smoothing() != TrendSmoothing::None,
                            unavailable: (!fits(option)).then(|| String::from("Zoom in to see more")),
                            onclick: move |_| graph_period.set(option),
                        }
                    }
                }
                if view() == ConsumptionView::Trend {
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
                    filter: category_filter,
                    disabled: categories.is_empty(),
                    categories,
                    years: range_years(&start, &end),
                    trend: view() == ConsumptionView::Trend,
                }
            }
            ExportButton {
                default_name: export_name,
                disabled: exportable.read().is_none(),
                table: export_table,
            }
        }
        {detail}
    }
}

#[derive(Props, Clone)]
struct MeasurementsDetailProps {
    // property: Arc<graphql::meter::account_properties_meters::MeterType>,
    account_id: String,
    mpan: String,
    export: bool,
    // device: Option<MeasurementsDevice>,
    billing_timezone: &'static Tz,
    start: DateTime,
    /// Exclusive end of the range
    end: DateTime,
    view: ConsumptionView,
    graph_mode: StackedBarMode,
    graph_period: StackedBarPeriod,
    smoothing: TrendSmoothing,
    category_filter: CategoryFilter,
    /// Set to the data once it has loaded, so that the parent can export it.
    exportable: Signal<Option<Arc<MeasurementsDataSet>>>,
    /// Called with the start and end of a bar in the graph when it is double clicked, None if there is no more detail.
    on_drill: Option<EventHandler<(i64, i64)>>,
    /// Called with the start and end of the bars dragged across in the graph.
    on_zoom: EventHandler<(i64, i64)>,
    /// The items in the menu shown when a bar is right clicked which change the range.
    range_menu: Vec<BarMenuItem>,
}

#[derive(Clone, Copy, PartialEq)]
enum ConsumptionView {
    Table,
    Graph,
    /// A line for each year, to compare years.
    Trend,
}

impl std::fmt::Display for ConsumptionView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsumptionView::Table => write!(f, "Table"),
            ConsumptionView::Graph => write!(f, "Graph"),
            ConsumptionView::Trend => write!(f, "Trend"),
        }
    }
}

impl PartialEq for MeasurementsDetailProps {
    fn eq(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.mpan == other.mpan
            && self.export == other.export
            // && self.device == other.device
            && self.start == other.start
            && self.end == other.end
            && self.view == other.view
            && self.graph_mode == other.graph_mode
            && self.graph_period == other.graph_period
            && self.smoothing == other.smoothing
            && self.category_filter == other.category_filter
            && self.exportable == other.exportable
            && self.on_drill.is_some() == other.on_drill.is_some()
            && self.range_menu.iter().map(|item| item.enabled).eq(other.range_menu.iter().map(|item| item.enabled))
            // && Arc::ptr_eq(&self.property, &other.property)
            && std::ptr::eq(self.billing_timezone, other.billing_timezone)
    }
}

#[component]
fn MeasurementsDetail(props: MeasurementsDetailProps) -> Element {
    let context = use_context::<Arc<OctopusContext>>();

    // The latest progress message from the fetch.
    let progress = use_signal(String::new);

    // Set by the cancel buttons, the fetch then stops and returns the data it has so far. Each reload has a new one, so
    // that cancelling a reload does not cancel the next.
    let mut cancel = use_signal(CancelToken::new);
    // Set when a Cancel button is clicked, so that it shows the fetch is stopping, until the next fetch starts.
    let mut cancelling = use_signal(|| false);

    // A period to fetch again from the server, however up to date the cache is, set from the bar menu.
    let mut reload = use_signal(|| None::<(DateTime, DateTime)>);

    // Fetch the data, and again whenever a period is reloaded.
    let transactions_resource = use_resource({
        let props = props.clone();
        let context = context.clone();
        move || {
            let props = props.clone();
            let cancel = cancel.peek().clone();
            let context = context.clone();
            let reload = reload();
            async move {
                cancelling.set(false);
                let monitor = move |message: String| { let mut progress = progress; progress.set(message) };
                MeasurementsDataSet::new(&props.account_id, &props.mpan, props.export, /* props.device.clone(), */
                    &props.start, &props.end, reload,
                    props.billing_timezone, &monitor,
                    &cancel, &context.cache_config, &*context.request_manager).await.map(Arc::new)
            }
        }
    });

    let tz = props.billing_timezone;
    let to_date_time = move |timestamp: i64| DateTime::from_unix_timestamp(timestamp).ok().map(|date_time| date_time.to_timezone(tz));

    // The menu shown when a bar is right clicked.
    let mut menu = props.range_menu.clone();
    for format in ExportFormat::ALL {
        let mpan = props.mpan.clone();
        menu.push(BarMenuItem {
            label: format!("Export this bar as {}", format.name()),
            enabled: true,
            action: EventHandler::new(move |click: BarClick| {
                let (Some(start), Some(end)) = (to_date_time(click.start), to_date_time(click.end)) else {
                    return;
                };
                let Some(Ok(data)) = &*transactions_resource.read() else {
                    return;
                };
                let table = data.readings_export(&start, &end);
                let name = export_file_name("measurements", &mpan, &start, &end, "readings");
                spawn(async move {
                    if let Err(error) = save_table(&table, &name, format).await {
                        crate::log_error!("Export of a bar failed: {error}");
                    }
                });
            }),
        });
    }
    menu.push(BarMenuItem {
        label: String::from("Reload this bar"),
        enabled: true,
        action: EventHandler::new(move |click: BarClick| {
            // The reload starts at the start of the bar before, not of this bar. The server has returned wrong data
            // near the start of a request, and starting the request a bar earlier keeps the bar being reloaded away from
            // it. Reloading 1 June 2026 this way repaired its data, which had been wrong.
            if let (Some(start), Some(end)) = (to_date_time(click.previous_start), to_date_time(click.end)) {
                cancel.set(CancelToken::new());
                reload.set(Some((start, end)));
            }
        }),
    });
    menu.push(BarMenuItem {
        label: String::from("Reload this month"),
        enabled: true,
        action: EventHandler::new(move |click: BarClick| {
            let Some(bar_start) = to_date_time(click.start) else {
                return;
            };
            let month = bar_start.to_date();
            let Ok(first) = sparko_graphql::types::Date::from_calendar_date(month.year(), month.month(), 1) else {
                return;
            };
            let next_month = if month.month() == time::Month::December {
                sparko_graphql::types::Date::from_calendar_date(month.year() + 1, time::Month::January, 1)
            }
            else {
                sparko_graphql::types::Date::from_calendar_date(month.year(), month.month().next(), 1)
            };
            // From the day before the month, for the same reason as Reload this bar starts at the bar before.
            if let Ok(next_month) = next_month {
                cancel.set(CancelToken::new());
                reload.set(Some((first.previous_day().at_midnight(tz), next_month.at_midnight(tz))));
            }
        }),
    });

    // Share the data with the parent for export once it has loaded.
    let mut exportable = props.exportable;
    use_effect(move || {
        let loaded = match &*transactions_resource.read() {
            Some(Ok(data)) => Some(data.clone()),
            _ => None,
        };
        exportable.set(loaded);
    });

    match &*transactions_resource.read() {
        None => rsx! {
            div {
                {format!("Loading measurements for meter {}...", props.mpan)}
                " "
                button {
                    r#type: "button",
                    disabled: cancelling(),
                    onclick: move |_| {
                        cancel.peek().cancel();
                        cancelling.set(true);
                    },
                    if cancelling() { "Cancelling…" } else { "Cancel" }
                }
            }
            div { "{progress}" }
        },
        // Errors from Octopus, e.g. that the account may not read a device, are explained here, others go to the
        // error page.
        Some(Err(error)) => match graphql_error_descriptions(error) {
            Some(descriptions) => rsx! {
                div { class: "error",
                    "Octopus did not provide the measurements for {props.mpan}:"
                    ul {
                        for description in descriptions {
                            li { "{description}" }
                        }
                    }
                }
            },
            None => Err(anyhow!("Failed to load measurements for meter {}: {error:#}", props.mpan))?,
        },
        Some(Ok(consumption)) => {
            let content = match props.view {
                ConsumptionView::Table => consumption.display(&props.start, &props.end),
                ConsumptionView::Graph => consumption.display_graph(
                    &props.start,
                    &props.end,
                    props.billing_timezone,
                    props.graph_mode,
                    props.graph_period,
                    &props.category_filter,
                    props.on_drill,
                    Some(props.on_zoom),
                    menu,
                ),
                ConsumptionView::Trend => consumption.display_trend(
                    &props.start,
                    &props.end,
                    props.billing_timezone,
                    props.graph_mode,
                    props.graph_period,
                    props.smoothing,
                    &props.category_filter,
                ),
            };
            // While a reload is fetched the data loaded before it is shown, with a dialog over it.
            let reloading = matches!(*transactions_resource.state().read(), UseResourceState::Pending);
            rsx! {
                div { class: "reloadable",
                    {content}
                    if reloading {
                        div { class: "reload-overlay",
                            div { class: "modal",
                                h3 { "Reloading" }
                                div { "{progress}" }
                                div { class: "modal-buttons",
                                    button {
                                        r#type: "button",
                                        disabled: cancelling(),
                                        onclick: move |_| {
                                            cancel.peek().cancel();
                                            cancelling.set(true);
                                        },
                                        if cancelling() { "Cancelling…" } else { "Cancel" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    }
}

impl ConsumptionView {
    /// What an export of this view contains, for the file name.
    fn export_description(&self, period: StackedBarPeriod) -> String {
        match self {
            ConsumptionView::Table => String::from("readings"),
            ConsumptionView::Graph => period.to_string().to_lowercase(),
            ConsumptionView::Trend => format!("trend_{}", period.to_string().to_lowercase()),
        }
    }
}
