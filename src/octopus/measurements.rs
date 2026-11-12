use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use dioxus::prelude::*;
use display_json::DisplayAsJsonPretty;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use bruce_graphql::types::{Currency, Date, DateTime, ForwardPageOf};
use bruce_graphql::{GraphQLCaller, GraphQLQueryBuilder, ProgressMonitor};
use time_tz::{TimeZone, Tz};

use crate::CacheConfig;
use crate::components::export::{ExportTable, ExportValue};
use crate::data_set::{CancelToken, MonthlyFixedSlotDataSet, SlotSource};
use crate::octopus::decimal::Decimal;
use crate::octopus::graphql::{ElectricityFiltersInput, ReadingFrequencyType, UtilityFiltersInput};

use crate::power::combine::MeterReading;
use super::graphql::meter;
use crate::components::category_filter::{CategoryFilter, CategoryValues, STANDING_CHARGE_LABEL, order_labels};
use crate::components::stacked_bar_graph::{BarMenuItem, GraphMode, day_bands, StackedBarGraph, StackedBarPeriod, StackedBarPoint, StackedBarSeries, time_tick_label};
use crate::components::category_filter::SOLID;
use super::trend_graph::{CATEGORY_LEGEND_COLOR, TrendGraph, TrendLegendItem, TrendLine, TrendSmoothing, cumulative_by_year, moving_average, range_years, year_positions};

/// The readings in a period of a graph added together, e.g. a day.
struct AggregatedPoint {
    start: i64,
    end: i64,
    tick_label: Option<String>,
    tick_rank: u8,
    period_label: String,
    /// The units of each label, in the order of the labels.
    units: Vec<f64>,
    /// The cost of each label, in the order of the labels.
    costs: Vec<f64>,
    standing_charge: f64,
}

impl AggregatedPoint {
    /// Add the values of another point with the same labels.
    fn add_values_from(&mut self, other: &Self) {
        for (total, value) in self.units.iter_mut().zip(&other.units) {
            *total += value;
        }
        for (total, value) in self.costs.iter_mut().zip(&other.costs) {
            *total += value;
        }
        self.standing_charge += other.standing_charge;
    }
}

/// The consumption labels used by any of the readings, see order_labels.
fn ordered_labels<'a>(readings: impl IntoIterator<Item = &'a MeasurementsReading>) -> Vec<String> {
    order_labels(readings.into_iter().flat_map(|reading| reading.consumption.keys().map(String::as_str)))
}


/// Measurement readings for a meter over a period, cached in monthly buckets.
pub struct MeasurementsDataSet {
    pub data: Vec<MonthlyFixedSlotDataSet<MeasurementsConsumptionSource>>,
}

// Readings of a smart device are not used, see the Smart Flex Devices list in pages/measurements.rs.
// /// A smart device whose readings are fetched instead of a meter point's, see SmartDevice.
// #[derive(Clone, PartialEq, Debug)]
// pub struct MeasurementsDevice {
    // pub id: String,
    // /// Whether the readings are the ones the server models for the device, rather than measured ones.
    // pub modelled: bool,
// }

impl MeasurementsDataSet {
    /// The readings of the meter point mpan.
    pub async fn new(
        account_number: &String,
        mpan: &String,
        export: bool,
        // device: Option<MeasurementsDevice>,
        start: &DateTime,
        end: &DateTime,
        reload: Option<(DateTime, DateTime)>,
        tz: &'static Tz,
        progress: &(impl ProgressMonitor + ?Sized),
        cancel: &CancelToken,
        cache_config: &Arc<CacheConfig>,
        request_manager: &impl GraphQLCaller)
        -> anyhow::Result<Self>
    {

        // let hash_key = match &device {
        //     Some(device) => format!("{}#{}#{}#{}#DeviceMeasurements", account_number, mpan, device.id, if device.modelled { "Modelled" } else { "Measured" }),
        //     None => format!("{}#{}#MeasurementsConsumption", account_number, mpan),
        // };
        let hash_key = format!("{}#{}#MeasurementsConsumption", account_number, mpan);
        let source = MeasurementsConsumptionSource{
            account_number: account_number.clone(),
            mpan: mpan.clone(),
            hash_key,
            tz,
            export,
            // device,
        };
        
        let data = MonthlyFixedSlotDataSet::fetch(
            source,
            tz,
            cache_config,
            request_manager,
            start,
            end,
            reload,
            cancel,
            progress,
        ).await?;

        Ok(Self { data })
    }

    /// The table of the readings from start to end, each as Octopus gave it if period is Raw, or else added up by
    /// period, as the graph's bars are, and so shown as derived, see aggregated_table.
    pub fn display(&self, start: &DateTime, end: &DateTime, tz: &'static Tz, period: StackedBarPeriod) -> Element {
        if period != StackedBarPeriod::Raw {
            return self.aggregated_table(start, end, tz, period);
        }
        let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
        let time_format = time::format_description::parse("[hour]:[minute]").unwrap();

        let rows: Vec<_> = self.data.iter()
            .flat_map(|data_set| data_set.data.range(start..))
            .filter(|(key, _)| *key < end)
            .collect();

        // A group of columns for each label used in the period.
        let labels = ordered_labels(rows.iter().map(|(_, item)| &item.value));

        let mut items = Vec::new();
        let mut last_date = Date::from_calendar_date(0, time::Month::January, 1)?;
        for (key, item) in rows {
            let row_class = if item.attributes.cached { "cached"} else {"fetched"};
            let item_start_date = item.value.start.to_date();
            let (start_date, start_time) = if last_date == item_start_date {
                ("".to_string(), item.value.start.format(&time_format).unwrap())
            } else {
                (item.value.start.format(&date_format).unwrap(), item.value.start.format(&time_format).unwrap())
            };

            let end_time = item.value.end.format(&time_format).unwrap();

            items.push(item.value.summary_line(&labels, row_class, &start_date, &start_time, &end_time));
            last_date = key.to_date();
        }

        rsx! {
            table {
                {MeasurementsReading::summary_headers(&labels)}
                for item in items {
                    {item}
                }
            }
        }
    }

    /// The readings from start to end added up by period, see graph_points, a row of each label's units, cost and
    /// average rate, its cost over its units, and the standing charge, all derived.
    fn aggregated_table(&self, start: &DateTime, end: &DateTime, tz: &'static Tz, period: StackedBarPeriod) -> Element {
        let (labels, points) = self.graph_points(start, end, tz, period);
        rsx! {
            table {
                tr {
                    td {}
                    for label in labels.iter() {
                        th { colspan: 3, class: "header", "{label}" }
                    }
                }
                tr { class: "header",
                    th { "{period}" }
                    for _ in labels.iter() {
                        th { "Units" }
                        th { "Cost" }
                        th { "Rate" }
                    }
                    th { "Standing Charge" }
                }
                for point in points.iter() {
                    tr {
                        td { "{point.period_label}" }
                        for (units, cost) in point.units.iter().zip(&point.costs) {
                            td { class: "derived", {format!("{units:.3}")} }
                            td { class: "derived", {format!("{cost:.2}")} }
                            td { class: "derived", if *units != 0.0 { {format!("{:.4}", cost / units)} } }
                        }
                        td { class: "derived", {format!("{:.2}", point.standing_charge)} }
                    }
                }
            }
        }
    }

    /// The units and cost of each label of each reading from start to end, leaving out readings with none, see
    /// is_partial, with the rate each is billed at, e.g. Low Rate for EV Low Rate and Home Low Rate, which are Octopus's
    /// split of that rate's units by use, the EV ones billed as the car's, as Octopus bills the car's charging apart, e.g.
    /// at the peak rate past a limit, and the standing charge. Costs are as Octopus gives them, so an export meter's
    /// are what it was paid.
    pub fn half_hours(&self, start: &DateTime, end: &DateTime) -> Vec<MeterReading> {
        let to_f64 = |value: &Decimal| value.to_f64().unwrap_or_default();
        self.readings(start, end)
            .filter(|reading| !reading.is_partial())
            .map(|reading| MeterReading {
                start: reading.start.unix_timestamp(),
                end: reading.end.unix_timestamp(),
                units: reading.consumption.iter().map(|(label, consumption)| (label.clone(), to_f64(&consumption.units))).collect(),
                rates: reading.consumption.keys()
                    .filter_map(|label| {
                        let rate = label.strip_prefix("Home ").or_else(|| label.strip_prefix("EV "))?;
                        Some((label.clone(), rate.to_string()))
                    })
                    .collect(),
                uses: reading.consumption.keys()
                    .filter(|label| label.starts_with("EV "))
                    .map(|label| (label.clone(), String::from(crate::power::topology::CAR)))
                    .collect(),
                costs: reading.consumption.iter().map(|(label, consumption)| (label.clone(), to_f64(&consumption.cost))).collect(),
                standing_charge: to_f64(&reading.standing_charge),
            })
            .collect()
    }

    /// The currency of the costs of the readings from start to end, None if none has one.
    pub fn currency(&self, start: &DateTime, end: &DateTime) -> Option<Currency> {
        self.readings(start, end).map(MeasurementsReading::currency).find(|currency| *currency != Currency::NONE)
    }

    /// The categories which can be shown in the graphs for the readings from start to end, the standing charge (for
    /// costs only) and then each label.
    pub fn categories(&self, start: &DateTime, end: &DateTime, mode: GraphMode) -> Vec<String> {
        let labels = ordered_labels(self.readings(start, end));
        Self::categories_for(&labels, mode)
    }

    fn categories_for(labels: &[String], mode: GraphMode) -> Vec<String> {
        let standing_charge = (mode == GraphMode::Cost).then(|| String::from(STANDING_CHARGE_LABEL));
        standing_charge.into_iter().chain(labels.iter().cloned()).collect()
    }

    /// The readings from start to end.
    fn readings<'a>(&'a self, start: &'a DateTime, end: &'a DateTime) -> impl Iterator<Item = &'a MeasurementsReading> + 'a {
        self.data.iter()
            .flat_map(move |data_set| data_set.data.range(start..))
            .filter(move |(key, _)| *key < end)
            .map(|(_, item)| &item.value)
    }

    /// The value of each category for each point, filtered by the filter for the trend graph if trend.
    /// Power is each point's average, its units over its hours.
    fn filtered_categories(labels: &[String], points: &[AggregatedPoint], mode: GraphMode, filter: &CategoryFilter, trend: bool) -> Vec<CategoryValues> {
        let categories = Self::categories_for(labels, mode).into_iter()
            .enumerate()
            .map(|(index, label)| {
                // In Cost mode the standing charge comes first, before the labels.
                let label_index = if mode == GraphMode::Cost { index.checked_sub(1) } else { Some(index) };
                let values = points.iter()
                    .map(|point| match (mode, label_index) {
                        (GraphMode::Cost, None) => point.standing_charge,
                        (GraphMode::Cost, Some(index)) => point.costs[index],
                        (GraphMode::Energy, Some(index)) => point.units[index],
                        (GraphMode::Power, Some(index)) => point.units[index] * 3600.0 / (point.end - point.start).max(1) as f64,
                        (GraphMode::Energy | GraphMode::Power, None) => 0.0,
                    })
                    .collect();
                (label, values)
            })
            .collect();
        filter.apply(categories, points.len(), trend)
    }

    /// The unit of consumption and the currency of the readings, e.g. kwh and GBP.
    fn graph_units(&self, start: &DateTime, end: &DateTime) -> (String, Currency) {
        let readings: Vec<&MeasurementsReading> = self.readings(start, end).collect();
        let unit = readings.iter()
            .map(|reading| reading.unit.as_str())
            .find(|unit| !unit.is_empty())
            .unwrap_or_default()
            .to_string();
        let currency = readings.iter()
            .map(|reading| reading.currency())
            .find(|currency| *currency != Currency::NONE)
            .unwrap_or(Currency::NONE);
        (unit, currency)
    }

    /// on_drill is called with the start and end of a bar when it is double clicked, and on_zoom with the start and end
    /// of the bars dragged across.
    pub fn display_graph(&self, start: &DateTime, end: &DateTime, tz: &'static Tz, mode: GraphMode, period: StackedBarPeriod, filter: &CategoryFilter,
        on_drill: Option<EventHandler<(i64, i64)>>, on_zoom: Option<EventHandler<(i64, i64)>>, menu: Vec<BarMenuItem>) -> Element {
        let (labels, aggregated) = self.graph_points(start, end, tz, period);
        let categories = Self::filtered_categories(&labels, &aggregated, mode, filter, false);
        let (unit, currency) = self.graph_units(start, end);

        let series = categories.iter()
            .map(|category| StackedBarSeries { label: category.label.clone(), color: category.color.clone() })
            .collect();
        let points = aggregated.into_iter()
            .enumerate()
            .map(|(index, point)| StackedBarPoint {
                detail: None,
                start: point.start,
                end: point.end,
                tick_label: point.tick_label,
                tick_rank: point.tick_rank,
                period_label: point.period_label,
                values: categories.iter().map(|category| category.values[index]).collect(),
            })
            .collect::<Vec<StackedBarPoint>>();
        // Raw readings and rate periods over more than a day have a band for each day under them, rather than labels at
        // the start of each day, a single day keeps its labels for the hours.
        let x_bands = match (period, points.first(), points.last()) {
            (StackedBarPeriod::Raw | StackedBarPeriod::Rate, Some(first), Some(last))
                if DateTime::from_unix_timestamp(first.start).map(|time| time.to_timezone(tz).to_date()).ok()
                    != DateTime::from_unix_timestamp(last.end - 1).map(|time| time.to_timezone(tz).to_date()).ok() =>
                day_bands(first.start, last.end, tz),
            _ => Vec::new(),
        };

        rsx! {
            StackedBarGraph {
                points,
                series,
                mode: mode.bar_mode(),
                currency,
                unit: if mode == GraphMode::Power { String::from("kW") } else { unit },
                // Rate periods have different lengths, so their energy and cost are per hour, power is already.
                per_hour: period == StackedBarPeriod::Rate && mode != GraphMode::Power,
                x_bands,
                on_drill,
                on_zoom,
                menu,
            }
        }
    }

    /// A line graph over one year, with a line for each category (or the total) in each year from start to end.
    /// smoothing other than None is worked out from daily totals, so period is then not used. The page does not offer
    /// Power here, as a cumulative or average of averages would mean little.
    pub fn display_trend(&self, start: &DateTime, end: &DateTime, tz: &'static Tz, mode: GraphMode, period: StackedBarPeriod,
        smoothing: TrendSmoothing, filter: &CategoryFilter) -> Element {
        let period = if smoothing == TrendSmoothing::None { period } else { StackedBarPeriod::Day };
        let (labels, aggregated) = self.graph_points(start, end, tz, period);
        let categories = Self::filtered_categories(&labels, &aggregated, mode, filter, true);
        let (unit, currency) = self.graph_units(start, end);
        let per_hour = period == StackedBarPeriod::Rate;

        // The year of each point and the position of its middle in that year.
        let positions = year_positions(aggregated.iter().map(|point| (point.start, point.end)), tz)?;
        let point_years: Vec<i32> = positions.iter().map(|(year, _)| *year).collect();
        let times: Vec<i64> = aggregated.iter().map(|point| point.start).collect();

        // The values to draw for a category, one for each point.
        let line_values = |values: &[f64]| -> Vec<f64> {
            match smoothing {
                TrendSmoothing::None => aggregated.iter()
                    .zip(values)
                    .map(|(point, value)| {
                        // Rate periods have different lengths, so their values are per hour.
                        let scale = if per_hour { 3600.0 / (point.end - point.start).max(1) as f64 } else { 1.0 };
                        value * scale
                    })
                    .collect(),
                TrendSmoothing::MovingAverage(days) => moving_average(&times, values, days),
                TrendSmoothing::Cumulative => cumulative_by_year(&point_years, values),
            }
        };
        let suffix = match smoothing {
            TrendSmoothing::None if per_hour => "/h",
            TrendSmoothing::MovingAverage(_) => "/day",
            _ => "",
        };

        // Each year has its own colour and each category its own line style, a single category is a solid line. The
        // colours are for every year in the range, as in the category menu, so that they match it.
        let years = range_years(start, end);
        let year_colors = filter.year_colors(&years);
        let single = categories.len() == 1;
        let mut lines = Vec::new();
        for (year, color) in years.iter().zip(&year_colors) {
            for category in &categories {
                let points: Vec<(f64, f64)> = positions.iter()
                    .zip(line_values(&category.values))
                    .filter(|((point_year, _), _)| point_year == year)
                    .map(|((_, position), value)| (*position, value))
                    .collect();
                if !points.is_empty() {
                    let dash = if single { String::from(SOLID) } else { category.dash.clone() };
                    let label = if single { year.to_string() } else { format!("{year} {}", category.label) };
                    lines.push(TrendLine { label, color: color.clone(), dash, points });
                }
            }
        }

        // The legend shows the line style of each category, unless there is only one, and the colour of each year.
        let shown_years: BTreeSet<i32> = positions.iter().map(|(year, _)| *year).collect();
        let legend = categories.iter()
            .filter(|_| !single)
            .map(|category| TrendLegendItem { label: category.label.clone(), color: String::from(CATEGORY_LEGEND_COLOR), dash: category.dash.clone() })
            .chain(years.iter().zip(&year_colors)
                .filter(|(year, _)| shown_years.contains(year))
                .map(|(year, color)| TrendLegendItem { label: year.to_string(), color: color.clone(), dash: String::from(SOLID) }))
            .collect();

        rsx! {
            TrendGraph {
                lines,
                legend,
                mode: mode.bar_mode(),
                currency,
                unit,
                suffix: String::from(suffix),
            }
        }
    }

    /// The number of bars in the graph from start to end by period.
    pub fn graph_bars(&self, start: &DateTime, end: &DateTime, tz: &'static Tz, period: StackedBarPeriod) -> usize {
        self.graph_points(start, end, tz, period).1.len()
    }

    /// The readings from start to end aggregated by period, with the labels in the order of their values.
    fn graph_points(&self, start: &DateTime, end: &DateTime, tz: &'static Tz, period: StackedBarPeriod) -> (Vec<String>, Vec<AggregatedPoint>) {
        let date_format = time::format_description::parse("[year]-[month]-[day]").unwrap();
        let day_format = time::format_description::parse("[day] [month repr:short]").unwrap();
        let time_format = time::format_description::parse("[hour]:[minute]").unwrap();
        let month_format = time::format_description::parse("[month repr:short] [year]").unwrap();

        let values: Vec<_> = self.data.iter()
            .flat_map(|data_set| data_set.data.range(start..))
            .filter(|(key, _)| *key < end)
            .map(|(_, item)| &item.value)
            .collect();

        // The values of each label used in the period.
        let labels = ordered_labels(values.iter().copied());

        let mut points = Vec::with_capacity(values.len());
        let single_day = matches!(period, StackedBarPeriod::Raw | StackedBarPeriod::Rate)
            && values.first().map(|value| value.start.to_date())
                == values.last().map(|value| value.start.to_date());
        let mut last_date = None;
        let mut last_bucket_start = None;

        // For Rate, the rates and start of the run of readings in the last point, and the date of the last run start.
        let mut last_rates = None;
        let mut run_start = None;
        let mut last_run_date = None;
        let range_label = |from: &DateTime, to: &DateTime| {
            if from.to_date() == to.to_date() || *to == from.to_date().next_day().at_midnight(tz) {
                format!("{} {}–{}", from.format(&date_format).unwrap(), from.format(&time_format).unwrap(), to.format(&time_format).unwrap())
            }
            else {
                format!("{} {} – {} {}", from.format(&date_format).unwrap(), from.format(&time_format).unwrap(),
                    to.format(&date_format).unwrap(), to.format(&time_format).unwrap())
            }
        };

        for value in values {
            let date = value.start.to_date();
            let (bucket_start, bucket_end) = match period {
                StackedBarPeriod::Raw | StackedBarPeriod::Rate => (value.start, value.end),
                StackedBarPeriod::Day => {
                    let bucket_start = date.at_midnight(tz);
                    (bucket_start, date.next_day().at_midnight(tz))
                }
                StackedBarPeriod::Week => {
                    let days_since_monday = date.weekday().number_days_from_monday() as i64;
                    let week_start = time::Date::from_calendar_date(
                        date.year(),
                        date.month(),
                        date.day(),
                    )
                    .unwrap()
                        - time::Duration::days(days_since_monday);
                    let week_start = Date::from_calendar_date(
                        week_start.year(),
                        week_start.month(),
                        week_start.day(),
                    )
                    .unwrap();
                    let week_end = week_start.next_day().next_day().next_day().next_day()
                        .next_day().next_day().next_day();
                    (week_start.at_midnight(tz), week_end.at_midnight(tz))
                }
                StackedBarPeriod::Month => {
                    let month_start = time::Date::from_calendar_date(
                        date.year(),
                        date.month(),
                        1,
                    )
                    .unwrap();
                    let next_month = if date.month() == time::Month::December {
                        time::Date::from_calendar_date(date.year() + 1, time::Month::January, 1)
                    } else {
                        time::Date::from_calendar_date(
                            date.year(),
                            time::Month::try_from(date.month() as u8 + 1).unwrap(),
                            1,
                        )
                    }
                    .unwrap();
                    let start_date = Date::from_calendar_date(
                        month_start.year(),
                        month_start.month(),
                        month_start.day(),
                    )
                    .unwrap();
                    let end_date = Date::from_calendar_date(
                        next_month.year(),
                        next_month.month(),
                        next_month.day(),
                    )
                    .unwrap();
                    (start_date.at_midnight(tz), end_date.at_midnight(tz))
                }
            };
            let (tick_label, tick_rank) = match period {
                StackedBarPeriod::Raw if single_day => time_tick_label(value.start.unix_timestamp(), value.end.unix_timestamp(), tz),
                _ => (None, 0),
            };
            let tick_label = match period {
                StackedBarPeriod::Raw if single_day => tick_label,
                StackedBarPeriod::Raw => {
                    (last_date != Some(date)).then(|| date.format(&day_format).unwrap())
                }
                // Set below when a new run starts.
                StackedBarPeriod::Rate => None,
                StackedBarPeriod::Day => {
                    (last_bucket_start != Some(bucket_start.unix_timestamp()))
                        .then(|| bucket_start.format(&day_format).unwrap())
                }
                StackedBarPeriod::Week => {
                    (last_bucket_start != Some(bucket_start.unix_timestamp())).then(|| {
                        format!("Week of {}", bucket_start.format(&day_format).unwrap())
                    })
                }
                StackedBarPeriod::Month => {
                    (last_bucket_start != Some(bucket_start.unix_timestamp()))
                        .then(|| bucket_start.format(&month_format).unwrap())
                }
            };
            let group_label = match period {
                StackedBarPeriod::Raw => format!(
                    "{} {}–{}",
                    value.start.format(&date_format).unwrap(),
                    value.start.format(&time_format).unwrap(),
                    value.end.format(&time_format).unwrap(),
                ),
                StackedBarPeriod::Rate => range_label(&value.start, &value.end),
                StackedBarPeriod::Day => bucket_start.format(&date_format).unwrap(),
                StackedBarPeriod::Week => format!(
                    "{} – {}",
                    bucket_start.format(&date_format).unwrap(),
                    bucket_end.to_date().previous_day().format(&date_format).unwrap(),
                ),
                StackedBarPeriod::Month => bucket_start.format(&month_format).unwrap(),
            };
            let point = AggregatedPoint {
                start: bucket_start.unix_timestamp(),
                end: bucket_end.unix_timestamp(),
                tick_label,
                tick_rank,
                period_label: group_label,
                units: labels.iter()
                    .map(|label| value.consumption.get(label).and_then(|c| c.units.to_f64()).unwrap_or_default())
                    .collect(),
                costs: labels.iter()
                    .map(|label| value.consumption.get(label).and_then(|c| c.cost.to_f64()).unwrap_or_default())
                    .collect(),
                standing_charge: value.standing_charge.to_f64().unwrap_or_default(),
            };

            if matches!(period, StackedBarPeriod::Raw) {
                points.push(point);
            } else if period == StackedBarPeriod::Rate {
                // Extend the last run if this reading follows on from it with the same rates, otherwise start a new run.
                let rates = value.rates();
                match points.last_mut() {
                    Some(existing) if last_rates.as_ref() == Some(&rates) && existing.end == point.start => {
                        existing.end = point.end;
                        existing.add_values_from(&point);
                        if let Some(run_start) = &run_start {
                            existing.period_label = range_label(run_start, &value.end);
                        }
                    },
                    _ => {
                        let mut point = point;
                        (point.tick_label, point.tick_rank) = if single_day {
                            time_tick_label(value.start.unix_timestamp(), value.end.unix_timestamp(), tz)
                        }
                        else {
                            ((last_run_date != Some(date)).then(|| date.format(&day_format).unwrap()), 0)
                        };
                        run_start = Some(value.start);
                        last_run_date = Some(date);
                        points.push(point);
                    },
                }
                last_rates = Some(rates);
            } else if let Some(existing) = points.last_mut() {
                if existing.start == bucket_start.unix_timestamp() {
                    existing.add_values_from(&point);
                } else {
                    points.push(point);
                }
            } else {
                points.push(point);
            }
            last_date = Some(date);
            last_bucket_start = Some(bucket_start.unix_timestamp());
        }

        (labels, points)
    }

    /// The readings from start to end, as shown in the table.
    pub fn readings_export(&self, start: &DateTime, end: &DateTime) -> ExportTable {
        let readings: Vec<&MeasurementsReading> = self.data.iter()
            .flat_map(|data_set| data_set.data.range(start..))
            .filter(|(key, _)| *key < end)
            .map(|(_, item)| &item.value)
            .collect();
        let labels = ordered_labels(readings.iter().copied());
        let (unit, cost_unit) = Self::export_units(&readings);

        // All the costs, then all the units, then the rates.
        let mut columns = vec![String::from("Start"), String::from("End")];
        columns.extend(labels.iter().map(|label| format!("{label} cost ({cost_unit})")));
        columns.push(format!("Standing charge ({cost_unit})"));
        columns.extend(labels.iter().map(|label| format!("{label} units ({unit})")));
        columns.extend(labels.iter().map(|label| format!("{label} rate ({cost_unit}/{unit})")));

        let decimal = |value: &Decimal| value.to_f64().map(ExportValue::number).unwrap_or(ExportValue::Empty);
        let rows = readings.iter()
            .map(|reading| {
                // The value of a field of each label's consumption, empty if the reading does not have that label.
                let field = |get: fn(&MeasurementsReadingConsumption) -> &Decimal| labels.iter()
                    .map(move |label| reading.consumption.get(label).map(|c| decimal(get(c))).unwrap_or(ExportValue::Empty));

                let mut row = vec![ExportValue::time(&reading.start), ExportValue::time(&reading.end)];
                row.extend(field(|c| &c.cost));
                row.push(decimal(&reading.standing_charge));
                row.extend(field(|c| &c.units));
                row.extend(field(|c| &c.rate));
                row
            })
            .collect();

        ExportTable { columns, rows }
    }

    /// The readings from start to end aggregated by period, as shown in the graph.
    pub fn graph_export(&self, start: &DateTime, end: &DateTime, tz: &'static Tz, period: StackedBarPeriod) -> ExportTable {
        let readings: Vec<&MeasurementsReading> = self.data.iter()
            .flat_map(|data_set| data_set.data.range(start..))
            .filter(|(key, _)| *key < end)
            .map(|(_, item)| &item.value)
            .collect();
        let (unit, cost_unit) = Self::export_units(&readings);
        let (labels, points) = self.graph_points(start, end, tz, period);

        // All the costs, then all the units.
        let mut columns = vec![String::from("Start"), String::from("End")];
        columns.extend(labels.iter().map(|label| format!("{label} cost ({cost_unit})")));
        columns.push(format!("Standing charge ({cost_unit})"));
        columns.extend(labels.iter().map(|label| format!("{label} units ({unit})")));

        let rows = points.iter()
            .map(|point| {
                let mut row = vec![ExportValue::unix_time(point.start, tz), ExportValue::unix_time(point.end, tz)];
                row.extend(point.costs.iter().map(|cost| ExportValue::number(*cost)));
                row.push(ExportValue::number(point.standing_charge));
                row.extend(point.units.iter().map(|units| ExportValue::number(*units)));
                row
            })
            .collect();

        ExportTable { columns, rows }
    }

    /// The units of consumption and of cost for column headings, e.g. ("kwh", "p").
    fn export_units(readings: &[&MeasurementsReading]) -> (String, String) {
        let unit = readings.iter()
            .map(|reading| reading.unit.as_str())
            .find(|unit| !unit.is_empty())
            .unwrap_or_default()
            .to_string();
        let currency = readings.iter()
            .map(|reading| reading.currency())
            .find(|currency| *currency != Currency::NONE)
            .unwrap_or(Currency::NONE);
        // Costs are in the minor units of the currency.
        let cost_unit = currency.subunit_symbol().unwrap_or_else(|| currency.code().to_string());
        (unit, cost_unit)
    }
}

#[derive(Serialize, Deserialize, Debug, DisplayAsJsonPretty, Eq, PartialEq, Clone)]
pub struct MeasurementsReadingConsumption {
    units: Decimal,
    cost:  Decimal,
    rate: Decimal,
    ccy: bruce_graphql::types::Currency,
    /// Whether the units were worked out from the cost and the rate, rather than given, as in the old format, see
    /// from_tou, so that the table shows them as derived. Left out when false, so that a reading's serialized form,
    /// which is compared to detect changes, is as it was.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    units_derived: bool,
    /// Whether the rate was worked out, e.g. from the cost and the units, see from_tou and GenerationValue in from.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    rate_derived: bool,
}

impl MeasurementsReadingConsumption {
    fn from(stat: meter::home_and_ev_consumption_breakdown::StatisticOutput) -> Self {


        let ccy = if let Some(ex_cost) = &stat.cost_excl_tax_ {
            ex_cost.cost_currency_
        }
        else {
            if let Some(inc_cost) = &stat.cost_incl_tax_ {
                inc_cost.cost_currency_
            }
            else {
                bruce_graphql::types::Currency::NONE
            }
        };

        let cost;
        let rate;

        if let Some(estimated_money) = stat.cost_incl_tax_ {
            cost = estimated_money.estimated_amount_;
            if let Some(price_per_unit) = estimated_money.price_per_unit_ {
                rate = price_per_unit.amount_;
            }
            else {
                rate = Decimal::ZERO;
            }
        }
        else {
            cost = Decimal::ZERO;
            rate = Decimal::ZERO;
        }

        let units = if let Some(units) = stat.value_ {
            units
        }
        else {
            Decimal::ZERO
        };

        MeasurementsReadingConsumption {
            units,
            cost,
            rate,
            ccy,
            units_derived: false,
            rate_derived: false,
        }
    }

    fn from_tou(stat: meter::home_and_ev_consumption_breakdown::StatisticOutput) -> anyhow::Result<Self> {
        let ccy = if let Some(ex_cost) = &stat.cost_excl_tax_ {
            ex_cost.cost_currency_
        }
        else {
            if let Some(inc_cost) = &stat.cost_incl_tax_ {
                inc_cost.cost_currency_
            }
            else {
                bruce_graphql::types::Currency::NONE
            }
        };

        let excl = stat.cost_excl_tax_.as_ref()
            .ok_or_else(|| anyhow!("TouBucketCost with no costExclTax"))?;
        let price_excl = excl.price_per_unit_.as_ref()
            .ok_or_else(|| anyhow!("TouBucketCost with no pricePerUnit"))?
            .amount_;
        let cost_excl = excl.estimated_amount_;
        let cost_incl = stat.cost_incl_tax_.as_ref()
            .map(|c| c.estimated_amount_)
            .unwrap_or(cost_excl);

        let units = if price_excl.is_zero() {
            Decimal::ZERO
        } else {
            (cost_excl / price_excl).normalize()
        };

        // Express the rate incl. tax, to match your cost column and the new format.
        // A zero-cost slot gives no tax ratio, so fall back to the excl price.
        // Units are 0 there, so it only affects the rate shown.
        let rate = if cost_excl.is_zero() {
            price_excl
        } else {
            (price_excl * cost_incl / cost_excl).round_dp(4)
        };

        Ok(MeasurementsReadingConsumption { units, cost: cost_incl, rate, ccy, units_derived: true, rate_derived: true })
    }
}

#[derive(Serialize, Deserialize, Debug, DisplayAsJsonPretty, Eq, PartialEq, Clone)]
pub struct MeasurementsReading {
    start:  DateTime,
    end: DateTime,
    is_final: bool,
    /// The consumption at each rate, indexed by the label from the API. A BTreeMap so that the serialized form, which
    /// is compared to detect changes, is always in the same order.
    consumption: BTreeMap<String, MeasurementsReadingConsumption>,
    standing_charge: Decimal,
    unit: String,
}

// /// The label of the consumption of a device reading which has no statistics, see MeasurementsReading::from.
// const DEVICE_LABEL: &str = "Device";

impl MeasurementsReading {
    const FINAL_DELAY: Duration = Duration::from_hours(3 * 24);

    /// Whether the reading has no statistics, which the server leaves out of every reading in a response if any day in
    /// it has a problem. Every statistic sets the consumption or the standing charge, so a reading with statistics
    /// has one or the other.
    fn is_partial(&self) -> bool {
        self.consumption.is_empty() && self.standing_charge.is_zero()
    }

    /// The unit rate for each label, readings with the same rates are grouped by StackedBarPeriod::Rate.
    fn rates(&self) -> BTreeMap<String, Decimal> {
        self.consumption.iter()
            .map(|(label, consumption)| (label.clone(), consumption.rate))
            .collect()
    }

    /// The currency of the costs, a consumption with no cost has no currency so the first real one is used.
    fn currency(&self) -> Currency {
        self.consumption.values()
            .map(|consumption| consumption.ccy)
            .find(|ccy| *ccy != Currency::NONE)
            .unwrap_or(Currency::NONE)
    }

    /// Add the consumption for a label, which must appear only once in a reading.
    fn insert_consumption(consumption: &mut BTreeMap<String, MeasurementsReadingConsumption>, label: String,
        value: MeasurementsReadingConsumption, start: &DateTime) -> anyhow::Result<()> {
        if consumption.contains_key(&label) {
            return Err(anyhow!("Duplicate {} consumption in slot {}", label, start));
        }
        consumption.insert(label, value);
        Ok(())
    }

    /// Table headers with a group of columns for each label.
    fn summary_headers(labels: &[String]) -> Element {
        rsx! {
            tr {
                td { colspan: 3 }
                for label in labels {
                    th { colspan: 3, class: "header", "{label}" }
                }
            }
            tr { class: "header",
                th { colspan: 2, "From" }
                th { "To" }
                for _ in labels {
                    th { "Units" }
                    th { "Cost" }
                    th { "Rate" }
                }
                th { "Standing Charge" }
            }
        }
    }

    /// A table row matching summary_headers, labels which this reading does not have are left empty.
    fn summary_line(&self, labels: &[String], row_class: &str, start_date: &str, start_time: &str, end_time: &str) -> Element {
        rsx!{
            tr { class: row_class,
                td { "{start_date}" }
                td { "{start_time}" }
                td { "{end_time}" }
                for label in labels {
                    if let Some(consumption) = self.consumption.get(label) {
                        td { class: if consumption.units_derived { "derived" } else { "" }, "{consumption.units}" }
                        td { "{consumption.cost}" }
                        td { class: if consumption.rate_derived { "derived" } else { "" }, "{consumption.rate}" }
                    }
                    else {
                        td {}
                        td {}
                        td {}
                    }
                }
                td { "{self.standing_charge}" }
            }
        }
    }
    /// A reading from the server.
    // For readings of a smart device, which are not used: a device's readings may have no statistics, or ones a meter
    // point's do not, so for a device statistics which are not understood are left out, and a reading without any
    // records its value as the consumption labelled DEVICE_LABEL.
    fn from(node: meter::home_and_ev_consumption_breakdown::MeasurementInterface, query_time: &DateTime /* , device: bool */) -> anyhow::Result<Self> {
        match node {
            meter::home_and_ev_consumption_breakdown::MeasurementInterface::MeasurementType(abstract_measurement_interface) => {

                Err(anyhow!(format!("Unexpected abstract measurement {:?}", abstract_measurement_interface)))
            },
            meter::home_and_ev_consumption_breakdown::MeasurementInterface::IntervalMeasurementType(interval_measurement_type) => {
                let statistics = match interval_measurement_type.measurement_interface_.meta_data_ {
                    Some(meta_data) => Some(meta_data.statistics_),
                    // None if device => Some(Vec::new()),
                    None => None,
                };
                if let Some(statistics) = statistics {
                    let mut consumption = BTreeMap::new();
                    let mut standing_charge = None;
                    let unit = interval_measurement_type.measurement_interface_.unit_;
                    // The total units of the measurement, e.g. the kWh exported.
                    let measured_value = interval_measurement_type.measurement_interface_.value_;

                    for stat in statistics {
                        if let Some(type_) = &stat.type_ {
                            match type_.clone() {
                                super::graphql::ReadingStatisticTypeEnum::StandingChargeCost => {
                                    if let Some(v) = stat.cost_incl_tax_ {
                                        standing_charge = Some(v.estimated_amount_)
                                    }
                                    else {
                                        return Err(anyhow!("Null standing charge"));
                                    }
                                },
                                // Consumption at one rate, labelled with the rate, e.g. "EV Low Rate".
                                super::graphql::ReadingStatisticTypeEnum::ConsumptionCost => {
                                    let label = stat.label_.clone()
                                        .ok_or_else(|| anyhow!("No meta data label"))?;
                                    Self::insert_consumption(&mut consumption, label, MeasurementsReadingConsumption::from(stat), &interval_measurement_type.start_at_)?;
                                },
                                // The old format, whole house consumption in time of use buckets, e.g. "OFF_PEAK".
                                super::graphql::ReadingStatisticTypeEnum::TouBucketCost => {
                                    let label = stat.label_.clone()
                                        .ok_or_else(|| anyhow!("No meta data label"))?;
                                    Self::insert_consumption(&mut consumption, label, MeasurementsReadingConsumption::from_tou(stat)?, &interval_measurement_type.start_at_)?;
                                },
                                // The value of what an export meter point exported, which may not have a label. The statistic only
                                // has the value, the units are those of the measurement and the rate is worked out from them.
                                super::graphql::ReadingStatisticTypeEnum::GenerationValue => {
                                    let label = stat.label_.clone().unwrap_or_else(|| String::from("Generation"));
                                    let mut generation = MeasurementsReadingConsumption::from(stat);
                                    generation.units = measured_value;
                                    if !measured_value.is_zero() {
                                        generation.rate = (generation.cost / measured_value).round_dp(4);
                                    }
                                    generation.rate_derived = true;
                                    Self::insert_consumption(&mut consumption, label, generation, &interval_measurement_type.start_at_)?;
                                },
                                // _ if device => {},
                                _ => {

                                    return Err(anyhow!(format!("Unexpected statistic type {:?}", type_)));
                                },
                            }
                        }
                        else {
                            return Err(anyhow!("No meta type"));
                        }
                    }

                    let standing_charge = if let Some(v) = standing_charge { v } else { Decimal::ZERO };

                    // if device && consumption.is_empty() {
                        // consumption.insert(String::from(DEVICE_LABEL), MeasurementsReadingConsumption {
                            // units: measured_value,
                            // cost: Decimal::ZERO,
                            // rate: Decimal::ZERO,
                            // ccy: Currency::NONE,
                        // });
                    // }

                    let mut reading = MeasurementsReading{
                        start: interval_measurement_type.start_at_,
                        end: interval_measurement_type.end_at_,
                        is_final: false,
                        consumption,
                        standing_charge,
                        unit
                    };
                    // A reading without statistics is fetched again until it has them, in case the server provides them later.
                    reading.is_final = !reading.is_partial()
                        && **query_time - *reading.end > MeasurementsReading::FINAL_DELAY;
                    Ok(reading)
                }
                else {
                    Err(anyhow!(format!("No meta data")))
                }
            },
        }
    }
}

pub struct MeasurementsConsumptionSource {
    pub account_number: String,
    pub tz: &'static Tz,
    pub mpan:String,
    pub hash_key: String,
    /// Whether the meter point is an export meter point, whose readings are generation rather than consumption.
    pub export: bool,
    // /// The device whose readings are fetched instead of the meter point's.
    // pub device: Option<MeasurementsDevice>,
}

impl MeasurementsConsumptionSource {
    /// A query for the readings up to end, the caller sets the start.
    fn query_builder(&self, end: &DateTime) -> anyhow::Result<meter::home_and_ev_consumption_breakdown::QueryBuilder> {
        let mut utility_filters = Vec::new();

        let electricity_filters = ElectricityFiltersInput::builder()
            .with_reading_frequency_type(ReadingFrequencyType::RawInterval)
            .with_reading_direction(if self.export {
                super::graphql::ReadingDirectionType::Generation
            } else {
                super::graphql::ReadingDirectionType::Consumption
            });
        let electricity_filters = electricity_filters.with_market_supply_point_id(self.mpan.clone());
        // A device is only linked to a property, not to a meter point, but the server may only find it within a
        // meter point, so it is filtered by both.
        // let electricity_filters = match &self.device {
        //     Some(device) => electricity_filters
        //         .with_device_id(device.id.clone())
        //         .with_modelled_sub_meter_readings(device.modelled),
        //     None => electricity_filters,
        // };
        utility_filters.push(UtilityFiltersInput::builder()
            .with_electricity_filters(electricity_filters.build()?)
            .build()?
        );

        // modelled_utility_filters.push(UtilityFiltersInput::builder()
        //     .with_electricity_filters(ElectricityFiltersInput::builder()
        //         .with_reading_frequency_type(ReadingFrequencyType::RawInterval)
        //         .with_reading_direction(super::graphql::ReadingDirectionType::Consumption)
        //         .with_market_supply_point_id(self.mpan.clone())
        //         .with_modelled_sub_meter_readings(true)
        //         .build()?)
        //     .build()?
        // );

        Ok(meter::home_and_ev_consumption_breakdown::Query::builder()
            .with_account_number(self.account_number.clone())
            .with_end_at(*end)
            .with_timezone(self.tz.name().to_string())
            .with_utility_filters(utility_filters)
            // .with_modelled_utility_filters(modelled_utility_filters)
            .with_first(1500)
        )
    }

    fn get_measurements(last_response: &meter::home_and_ev_consumption_breakdown::Response) -> anyhow::Result<&ForwardPageOf<meter::home_and_ev_consumption_breakdown::MeasurementInterface>> {
            Ok(&last_response
                .account_.as_ref()
                .ok_or(anyhow!("No account in response"))?
                .properties_.first().ok_or(anyhow!("No property in response"))?
                // .electricity_meter_points_.first().ok_or("No meter points in response")?
                .measurements_

            )
        }
}

/// A measurement as the store keeps it, see SlotSource::Stored: as Octopus gave it, with when it was fetched, which
/// with its time says whether it is final, see MeasurementsReading::from, or, from a profile made before the store, as
/// a reading an older version of the app made of it, whose figures it worked out are not marked as such.
#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum StoredMeasurement {
    Given {
        node: meter::home_and_ev_consumption_breakdown::MeasurementInterface,
        fetched: DateTime,
    },
    Converted(MeasurementsReading),
}

impl SlotSource for MeasurementsConsumptionSource {
    type Value = MeasurementsReading;
    type Stored = StoredMeasurement;

    type Response = meter::home_and_ev_consumption_breakdown::Response;

    type Query = meter::home_and_ev_consumption_breakdown::Query;

    fn hash_key(&self) -> &str {
        &self.hash_key
    }

    fn initial_query(&mut self, start: &DateTime, end: &DateTime) -> anyhow::Result<Self::Query> {
        Ok(self.query_builder(end)?
            .with_start_at(start.clone())
            .build()?
        )
    }

    fn continuation_query(&mut self, start: &DateTime, end: &DateTime, last_response: &Self::Response) -> anyhow::Result<Option<Self::Query>> {


        // fn get_cursor(last_response: &meter::home_and_ev_consumption_breakdown::Response) -> anyhow::Result<Option<String>> {
        //     Ok(last_response
        //         .account_.as_ref()
        //         .ok_or(anyhow!("No account in response"))?
        //         .properties_.first().ok_or(anyhow!("No property in response"))?
        //         // .electricity_meter_points_.first().ok_or("No meter points in response")?
        //         .measurements_.as_ref().ok_or(anyhow!("No measurements in response"))?
        //         .page_info_
        //         .end_cursor
        //         .as_ref()
        //         .cloned()
        //     )
        // }

        Ok(if let Some(end_cursor) = Self::get_measurements(&last_response)?
            .page_info
                .end_cursor
                .as_ref()
                .cloned()
        {
            Some(self.query_builder(end)?
                .with_start_at(start.clone())
                .with_cursor(end_cursor)
                .build()?)
        }
        else {
            None
        })
    }

    /// Each measurement as Octopus gave it, with when it was fetched, see StoredMeasurement.
    fn items(&self, response: Self::Response) -> anyhow::Result<impl IntoIterator<Item = anyhow::Result<Self::Stored>>> {
        let fetched = DateTime::now(self.tz);
        Ok(response
            .account_
            .ok_or(anyhow!("No account in response"))?
            .properties_.into_iter().next().ok_or(anyhow!("No property in response"))?
            .measurements_
            .edges
            .into_iter()
            .map(move |edge| Ok(StoredMeasurement::Given { node: edge.node, fetched: fetched.clone() })))
    }

    /// The reading, made from the measurement as Octopus gave it each time it is read, so that what was worked out,
    /// e.g. the units of the old format, is marked as such, see MeasurementsReadingConsumption.
    fn value(&self, stored: &Self::Stored) -> anyhow::Result<Self::Value> {
        match stored {
            StoredMeasurement::Given { node, fetched } => MeasurementsReading::from(node.clone(), fetched),
            StoredMeasurement::Converted(reading) => Ok(reading.clone()),
        }
    }

    fn stored_from_value(&self, value: serde_json::Value) -> Option<Self::Stored> {
        serde_json::from_value(value).ok().map(StoredMeasurement::Converted)
    }

    fn index(&self, value: &Self::Value) -> anyhow::Result<(DateTime, DateTime, bool)> {
        // Readings cached as final before readings without statistics were treated as not final are fetched again.
        Ok((value.start.clone(), value.end.clone(), value.is_final && !value.is_partial()))
    }
    
    fn refresh_after(&self) -> Duration {
        Duration::from_hours(24)
    }

    fn is_partial(&self, value: &Self::Value) -> bool {
        value.is_partial()
    }
}

#[cfg(test)]
mod tests {
    use super::{AggregatedPoint, order_labels};

    #[test]
    fn known_labels_come_first_in_their_usual_order_then_others_alphabetically() {
        let labels = order_labels(["STANDARD_RATE", "EV Standard Rate", "OFF_PEAK", "Home Low Rate", "EV Standard Rate"]);
        assert_eq!(labels, ["Home Low Rate", "EV Standard Rate", "OFF_PEAK", "STANDARD_RATE"]);
    }

    #[test]
    fn adding_points_sums_every_value() {
        let point = || AggregatedPoint {
            start: 0,
            end: 1,
            tick_label: None,
            tick_rank: 0,
            period_label: String::new(),
            units: vec![1.0, 2.0],
            costs: vec![5.0, 6.0],
            standing_charge: 9.0,
        };
        let mut total = point();
        total.add_values_from(&point());
        assert_eq!(total.units, [2.0, 4.0]);
        assert_eq!(total.costs, [10.0, 12.0]);
        assert_eq!(total.standing_charge, 18.0);
    }
}
