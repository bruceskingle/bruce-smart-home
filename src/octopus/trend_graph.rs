//! A line graph of values over one year from 1 January, with a line for each year so that years can be compared.

use anyhow::anyhow;
use dioxus::prelude::*;
use sparko_graphql::types::{Currency, Date, DateTime};
use time_tz::{OffsetDateTimeExt, Tz};

use crate::components::stacked_bar_graph::{StackedBarMode, axis_label, axis_unit, format_money, nice_step};

const LEFT: f64 = 64.0;
const RIGHT: f64 = 16.0;
const TOP: f64 = 44.0;
const BOTTOM: f64 = 32.0;
const Y_TICKS: f64 = 4.0;
const LEGEND_SPACING: f64 = 150.0;
const LEGEND_SAMPLE_LENGTH: f64 = 24.0;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// The day of the year each month starts on, in a year which is not a leap year.
const MONTH_START_DAYS: [f64; 12] = [0.0, 31.0, 59.0, 90.0, 120.0, 151.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0];

/// How the values in the trend graph are smoothed, all except None are worked out from daily totals.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TrendSmoothing {
    None,
    /// The average of the daily totals over this many days, centred on each day.
    MovingAverage(i64),
    /// The total of the daily totals so far in each year.
    Cumulative,
}

impl TrendSmoothing {
    pub const ALL: [TrendSmoothing; 4] = [
        TrendSmoothing::None,
        TrendSmoothing::MovingAverage(7),
        TrendSmoothing::MovingAverage(30),
        TrendSmoothing::Cumulative,
    ];
}

impl std::fmt::Display for TrendSmoothing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrendSmoothing::None => write!(f, "None"),
            TrendSmoothing::MovingAverage(days) => write!(f, "{days} day"),
            TrendSmoothing::Cumulative => write!(f, "Cumulative"),
        }
    }
}

/// The average of the values within half of window_days either side of each one, by time. times are unix timestamps
/// in order, and days without a value are left out of the average rather than counted as zero.
pub fn moving_average(times: &[i64], values: &[f64], window_days: i64) -> Vec<f64> {
    let half_window = window_days * 86400 / 2;
    let mut averages = Vec::with_capacity(values.len());
    let (mut first, mut after_last, mut sum) = (0, 0, 0.0);
    for &time in times {
        while after_last < times.len() && times[after_last] <= time + half_window {
            sum += values[after_last];
            after_last += 1;
        }
        while times[first] < time - half_window {
            sum -= values[first];
            first += 1;
        }
        averages.push(sum / (after_last - first) as f64);
    }
    averages
}

/// The running total of the values, starting again whenever the year changes.
pub fn cumulative_by_year(years: &[i32], values: &[f64]) -> Vec<f64> {
    let mut totals = Vec::with_capacity(values.len());
    let mut total = 0.0;
    let mut current_year = None;
    for (year, value) in years.iter().zip(values) {
        if current_year != Some(*year) {
            current_year = Some(*year);
            total = 0.0;
        }
        total += value;
        totals.push(total);
    }
    totals
}

/// The number of columns the tooltip divides the year into, one for each day of a year which is not a leap year.
const DAYS: usize = 365;
const TOOLTIP_WIDTH: f64 = 260.0;
const TOOLTIP_TEXT_X: f64 = 32.0;
const TOOLTIP_ROW_HEIGHT: f64 = 14.0;

/// A label for a day of a year which is not a leap year, from 0 for 1 January, e.g. "14 Jun".
fn day_label(day: usize) -> String {
    let month = MONTH_START_DAYS.iter().rposition(|start| *start <= day as f64).unwrap_or(0);
    format!("{} {}", day + 1 - MONTH_START_DAYS[month] as usize, MONTHS[month])
}

/// The value of the point of a line nearest to position, if it is near enough to be the value there, which is within
/// a day or within the spacing of the point from its nearest neighbour, so that a point at the edge of a gap in the
/// data is not used for days in the gap. points are in order of position.
fn value_near(points: &[(f64, f64)], position: f64) -> Option<f64> {
    let after = points.partition_point(|(point_position, _)| *point_position < position);
    let nearest = [after.checked_sub(1), (after < points.len()).then_some(after)]
        .into_iter()
        .flatten()
        .min_by(|a, b| (points[*a].0 - position).abs().total_cmp(&(points[*b].0 - position).abs()))?;
    let spacing = [nearest.checked_sub(1), (nearest + 1 < points.len()).then_some(nearest + 1)]
        .into_iter()
        .flatten()
        .map(|other| (points[other].0 - points[nearest].0).abs())
        .reduce(f64::min)
        .unwrap_or_default()
        .max(1.0 / DAYS as f64);
    ((points[nearest].0 - position).abs() <= spacing).then_some(points[nearest].1)
}

/// The colour of the line style samples of categories in the legend, which show only the style.
pub const CATEGORY_LEGEND_COLOR: &str = "#c4c4ce";

/// The years in the range from start to end, which is exclusive, in order.
pub fn range_years(start: &DateTime, end: &DateTime) -> Vec<i32> {
    let last = **end - time::Duration::SECOND;
    (start.year()..=last.year()).collect()
}

/// The year of the middle of each period, given as its start and end unix timestamps, and the position of the middle in
/// that year, from 0 at the start of 1 January to 1 at the end of 31 December.
pub fn year_positions(periods: impl IntoIterator<Item = (i64, i64)>, tz: &'static Tz) -> anyhow::Result<Vec<(i32, f64)>> {
    periods.into_iter()
        .map(|(start, end)| {
            let middle = start + (end - start) / 2;
            let year = time::OffsetDateTime::from_unix_timestamp(middle)
                .map_err(|error| anyhow!(error))?
                .to_timezone(tz)
                .year();
            let year_start = Date::from_calendar_date(year, time::Month::January, 1)?.at_midnight(tz).unix_timestamp();
            let next_year_start = Date::from_calendar_date(year + 1, time::Month::January, 1)?.at_midnight(tz).unix_timestamp();
            Ok((year, (middle - year_start) as f64 / (next_year_start - year_start) as f64))
        })
        .collect()
}

#[derive(Clone, PartialEq)]
pub struct TrendLine {
    /// What the line is, for the tooltip, e.g. "2025 Home Low Rate".
    pub label: String,
    /// A CSS colour, e.g. #46ee43
    pub color: String,
    /// An SVG stroke-dasharray, e.g. "8 4", or "none" for a solid line.
    pub dash: String,
    /// The position of each point in the year, from 0 at the start of 1 January to 1 at the end of 31 December, and
    /// its value.
    pub points: Vec<(f64, f64)>,
}

/// An item in the legend, shown as a short sample of a line.
#[derive(Clone, PartialEq)]
pub struct TrendLegendItem {
    pub label: String,
    pub color: String,
    pub dash: String,
}

#[component]
pub fn TrendGraph(
    lines: Vec<TrendLine>,
    legend: Vec<TrendLegendItem>,
    /// Whether the values are units or costs, for formatting.
    mode: StackedBarMode,
    /// The currency of costs, which are in its minor units, e.g. pence
    currency: Currency,
    /// The unit of consumption, e.g. kwh
    unit: String,
    /// What the values are per, for the y axis labels, e.g. "/h" or "/day".
    #[props(default)] suffix: String,
) -> Element {
    let mut dimensions = use_signal(|| None::<(f64, f64)>);
    // The day of the year the mouse is over.
    let mut hovered = use_signal(|| None::<usize>);

    if lines.iter().all(|line| line.points.is_empty()) {
        return rsx! { p { "No data for this period" } };
    }

    let (width, height) = dimensions().unwrap_or((960.0, 320.0));
    let plot_width = (width - LEFT - RIGHT).max(1.0);
    let plot_height = (height - TOP - BOTTOM).max(1.0);
    let x = |position: f64| LEFT + position * plot_width;

    let max_value = lines.iter()
        .flat_map(|line| line.points.iter().map(|(_, value)| *value))
        .fold(0.0, f64::max);
    let (step, decimals) = nice_step(max_value / Y_TICKS);
    let y_max = (max_value / step).ceil().max(1.0) * step;
    let y = |value: f64| TOP + plot_height - value.max(0.0) / y_max * plot_height;

    let y_ticks: Vec<(f64, String)> = (0..)
        .map(|i| i as f64 * step)
        .take_while(|value| *value <= y_max + step / 2.0)
        .map(|value| (y(value), axis_label(value, decimals, mode, currency)))
        .collect();
    let y_unit = axis_unit(mode, currency, &unit, &suffix);

    // A tick at the start of each month, with the name of the month in the middle of it.
    let month_ticks: Vec<(f64, f64, &str)> = MONTHS.iter()
        .zip(MONTH_START_DAYS)
        .enumerate()
        .map(|(index, (name, start_day))| {
            let end_day = MONTH_START_DAYS.get(index + 1).copied().unwrap_or(365.0);
            (x(start_day / 365.0), x((start_day + end_day) / 2.0 / 365.0), *name)
        })
        .collect();

    let paths: Vec<(String, String, String)> = lines.iter()
        .filter(|line| !line.points.is_empty())
        .map(|line| {
            let points = line.points.iter()
                .map(|(position, value)| format!("{:.1},{:.1}", x(*position), y(*value)))
                .collect::<Vec<_>>()
                .join(" ");
            (points, line.color.clone(), line.dash.clone())
        })
        .collect();

    let legend: Vec<(f64, TrendLegendItem)> = legend.into_iter()
        .enumerate()
        .map(|(index, item)| (LEFT + index as f64 * LEGEND_SPACING, item))
        .collect();

    // A column for each day of the year, which shows the tooltip for that day when the mouse is over it.
    let column_width = plot_width / DAYS as f64;
    let format_value = |value: f64| match mode {
        StackedBarMode::Units => format!("{value:.3} {unit}{suffix}"),
        StackedBarMode::Cost => format!("{}{suffix}", format_money(value, currency)),
    };
    let tooltip = hovered().map(|day| {
        let position = (day as f64 + 0.5) / DAYS as f64;
        let line_x = x(position);
        let tooltip_x = if line_x > width / 2.0 { line_x - TOOLTIP_WIDTH - 10.0 } else { line_x + 10.0 }
            .clamp(LEFT, (width - TOOLTIP_WIDTH - RIGHT).max(LEFT));
        // The value of each line on that day, leaving out lines with no value or a value of zero.
        let rows: Vec<(String, String, String, String)> = lines.iter()
            .filter_map(|line| value_near(&line.points, position)
                .filter(|value| *value != 0.0)
                .map(|value| (line.label.clone(), line.color.clone(), line.dash.clone(), format_value(value))))
            .collect();
        (line_x, tooltip_x, day_label(day), rows)
    });

    rsx! {
        // The frame is measured, not the svg, because some versions of WebKit, e.g. on an Intel Mac, give the size of
        // what is drawn in an svg as its size, which is a little smaller than the svg, so each resize shrank the graph.
        div { class: "graph-frame",
            onresize: move |event| {
                match event.data().get_content_box_size() {
                    Ok(size) if size.width > 0.0 && size.height > 0.0 => {
                        dimensions.set(Some((size.width, size.height)));
                    }
                    Ok(_) => eprintln!("Trend graph resized to an empty size"),
                    Err(error) => eprintln!("Failed to read trend graph size: {error:?}"),
                }
            },
            svg {
                class: if dimensions().is_some() { "stacked-consumption-graph ready" } else { "stacked-consumption-graph" },
                view_box: "0 0 {width} {height}",
                role: "img",
                onmouseleave: move |_| hovered.set(None),
                // Keyed groups, see StackedBarGraph.
                for (index, (ty, label)) in y_ticks.into_iter().enumerate() {
                    g { key: "{index}",
                        line { class: "stacked-grid", x1: LEFT, x2: width - RIGHT, y1: ty, y2: ty }
                        text { class: "stacked-axis-label", x: LEFT - 8.0, y: ty, text_anchor: "end", dominant_baseline: "middle", "{label}" }
                    }
                }
                // The units of the y axis labels, above them in the row of the legend.
                text { class: "stacked-axis-label", x: LEFT - 8.0, y: 24.0, text_anchor: "end", "{y_unit}" }
                for (legend_x, item) in legend {
                    line {
                        class: "trend-line",
                        stroke: "{item.color}",
                        stroke_dasharray: "{item.dash}",
                        x1: legend_x,
                        x2: legend_x + LEGEND_SAMPLE_LENGTH,
                        y1: 20.0,
                        y2: 20.0,
                    }
                    text { class: "stacked-axis-label", x: legend_x + LEGEND_SAMPLE_LENGTH + 6.0, y: 24.0, "{item.label}" }
                }
                for (index, (tick_x, label_x, name)) in month_ticks.into_iter().enumerate() {
                    g { key: "{index}",
                        line { class: "stacked-tick", x1: tick_x, x2: tick_x, y1: TOP + plot_height, y2: TOP + plot_height + 4.0 }
                        text { class: "stacked-axis-label", x: label_x, y: height - 10.0, text_anchor: "middle", "{name}" }
                    }
                }
                for (points, color, dash) in paths {
                    polyline {
                        class: "trend-line",
                        points: "{points}",
                        stroke: "{color}",
                        stroke_dasharray: "{dash}",
                    }
                }
                if let Some((line_x, _, _, _)) = &tooltip {
                    line { class: "trend-crosshair", x1: *line_x, x2: *line_x, y1: TOP, y2: TOP + plot_height }
                }
                for day in 0..DAYS {
                    rect {
                        class: "trend-hit",
                        x: LEFT + day as f64 * column_width,
                        y: TOP,
                        width: column_width,
                        height: plot_height,
                        onmouseenter: move |_| hovered.set(Some(day)),
                    }
                }
                if let Some((_, tooltip_x, day, rows)) = tooltip {
                    g { class: "stacked-tooltip",
                        rect {
                            class: "stacked-tooltip-background",
                            x: tooltip_x,
                            y: TOP,
                            width: TOOLTIP_WIDTH,
                            height: 30.0 + rows.len() as f64 * TOOLTIP_ROW_HEIGHT,
                            rx: 4,
                        }
                        text { class: "stacked-tooltip-period", x: tooltip_x + 8.0, y: TOP + 16.0, "{day}" }
                        for (index, (label, color, dash, value)) in rows.iter().enumerate() {
                            line {
                                class: "trend-line",
                                stroke: "{color}",
                                stroke_dasharray: "{dash}",
                                x1: tooltip_x + 8.0,
                                x2: tooltip_x + TOOLTIP_TEXT_X - 6.0,
                                y1: TOP + 30.0 + index as f64 * TOOLTIP_ROW_HEIGHT,
                                y2: TOP + 30.0 + index as f64 * TOOLTIP_ROW_HEIGHT,
                            }
                            text {
                                class: "stacked-tooltip-value",
                                x: tooltip_x + TOOLTIP_TEXT_X,
                                y: TOP + 34.0 + index as f64 * TOOLTIP_ROW_HEIGHT,
                                "{label}: {value}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DAYS, cumulative_by_year, day_label, moving_average, value_near};

    const DAY: i64 = 86400;

    #[test]
    fn a_moving_average_is_centred_and_leaves_out_missing_days() {
        let times: Vec<i64> = (0..10).map(|day| day * DAY).collect();
        let values: Vec<f64> = (0..10).map(|day| day as f64).collect();
        let averages = moving_average(&times, &values, 3);
        // Day 0 has only days 0 and 1, day 5 has days 4, 5 and 6.
        assert_eq!(averages[0], 0.5);
        assert_eq!(averages[5], 5.0);
        assert_eq!(averages[9], 8.5);

        // With day 5 missing, day 6 averages days 6 and 7.
        let times = [4 * DAY, 6 * DAY, 7 * DAY];
        assert_eq!(moving_average(&times, &[4.0, 6.0, 7.0], 3), [4.0, 6.5, 6.5]);
    }

    #[test]
    fn a_cumulative_total_starts_again_each_year() {
        assert_eq!(
            cumulative_by_year(&[2025, 2025, 2025, 2026, 2026], &[1.0, 2.0, 3.0, 10.0, 20.0]),
            [1.0, 3.0, 6.0, 10.0, 30.0]
        );
    }

    #[test]
    fn days_of_the_year_are_labelled_with_the_date() {
        assert_eq!(day_label(0), "1 Jan");
        assert_eq!(day_label(31), "1 Feb");
        assert_eq!(day_label(164), "14 Jun");
        assert_eq!(day_label(364), "31 Dec");
    }

    #[test]
    fn the_value_near_a_day_is_the_nearest_point_unless_it_is_across_a_gap() {
        let day = 1.0 / DAYS as f64;
        // Daily points for days 0 to 9, then nothing until a point on day 200.
        let mut points: Vec<(f64, f64)> = (0..10).map(|d| ((d as f64 + 0.5) * day, d as f64)).collect();
        points.push((200.5 * day, 200.0));

        assert_eq!(value_near(&points, 3.5 * day), Some(3.0));
        assert_eq!(value_near(&points, 3.9 * day), Some(3.0));
        // Day 100 is in the gap, the nearest point is a long way off.
        assert_eq!(value_near(&points, 100.5 * day), None);
        assert_eq!(value_near(&[], 0.5), None);

        // Monthly points are used for any day of their month.
        let months: Vec<(f64, f64)> = (0..12).map(|m| ((m as f64 + 0.5) / 12.0, m as f64)).collect();
        assert_eq!(value_near(&months, 0.01), Some(0.0));
        assert_eq!(value_near(&months, 0.45), Some(5.0));
    }
}
