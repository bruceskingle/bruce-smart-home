use std::collections::BTreeSet;

use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;
use sparko_graphql::types::{Currency, DateTime};
use time_tz::Tz;

const LEFT: f64 = 64.0;
const RIGHT: f64 = 16.0;
const TOP: f64 = 44.0;
const BOTTOM: f64 = 32.0;
/// The space around an x axis label in its band.
const LABEL_PADDING: f64 = 4.0;
const Y_TICKS: f64 = 4.0;
const BAR_GAP: f64 = 2.0;
/// An estimate, a little on the wide side, of the width of a character of the 12px axis labels, to leave out x axis
/// labels which would not fit.
const LABEL_CHAR_WIDTH: f64 = 7.0;
const TOOLTIP_WIDTH: f64 = 226.0;
/// The tooltip text starts after a column of colour swatches.
const TOOLTIP_TEXT_X: f64 = 24.0;

#[derive(Clone, Copy, PartialEq)]
pub enum StackedBarMode {
    Units,
    Cost,
}

/// The most bars a graph should have, more make dragging across them to zoom too slow to follow. Pages disable the
/// periods that would have more.
pub const MAX_BARS: usize = 2200;

/// In order from the shortest, so that a page can show the next longer period when one is too short for its range.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StackedBarPeriod {
    Raw,
    /// Consecutive readings with the same unit rates, so the bars have different widths.
    Rate,
    Day,
    Week,
    Month,
}

impl std::fmt::Display for StackedBarMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cost => write!(f, "Cost"),
            Self::Units => write!(f, "Units"),
        }
    }
}

impl std::fmt::Display for StackedBarPeriod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw => write!(f, "Raw"),
            Self::Rate => write!(f, "Rate"),
            Self::Day => write!(f, "Day"),
            Self::Week => write!(f, "Week"),
            Self::Month => write!(f, "Month"),
        }
    }
}

/// One series stacked in each bar, e.g. the consumption at one rate.
#[derive(Clone, PartialEq)]
pub struct StackedBarSeries {
    pub label: String,
    /// A CSS colour, e.g. #46ee43
    pub color: String,
}

/// A line drawn over the bars, e.g. another estimate of one of the series, to compare with it, with a value for each
/// point, None where it has none, in the same units as their values, named in the legend and the tooltip. Dashed, or
/// solid, so that two lines of the same colour can be told apart.
#[derive(Clone, Debug, PartialEq)]
pub struct StackedBarLine {
    pub label: String,
    pub color: String,
    pub values: Vec<Option<f64>>,
    pub dashed: bool,
}

#[derive(Clone, PartialEq)]
pub struct StackedBarPoint {
    pub start: i64,
    pub end: i64,
    pub tick_label: Option<String>,
    /// How round the time of the tick label is, from time_tick, or 0 if they are all as good, e.g. days. When not all
    /// of the labels fit, those with the highest ranks are shown.
    pub tick_rank: u8,
    pub period_label: String,
    /// More about the point, for a line of the tooltip under period_label, e.g. where its values are from.
    pub detail: Option<String>,
    /// The value of each series, in the order of the graph's series, bottom first.
    pub values: Vec<f64>,
}

/// A bar which was right clicked, with times as unix timestamps.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BarClick {
    pub start: i64,
    pub end: i64,
    /// The start of the bar before it, or for the first bar the same length before it.
    pub previous_start: i64,
}

/// An item in the menu shown when a bar is right clicked.
#[derive(Clone, PartialEq)]
pub struct BarMenuItem {
    pub label: String,
    pub enabled: bool,
    pub action: EventHandler<BarClick>,
}

/// The height of an item in the bar menu and the width of a character of its labels, a little on the large side, and
/// its padding, to estimate its size.
const MENU_ITEM_HEIGHT: f64 = 30.0;
const MENU_CHAR_WIDTH: f64 = 8.0;
const MENU_PADDING: f64 = 40.0;

/// The CSS position of the bar menu for a right click at x, y in the window: below and to the right of the mouse, or
/// where that would go off the edge of the window, above or to the left of it. Then the menu is placed by its bottom or
/// right edge, so it is at the mouse however good the estimate of its size is.
fn menu_position(x: f64, y: f64, menu: &[BarMenuItem]) -> String {
    let window = dioxus::desktop::window();
    let size = window.window.inner_size().to_logical::<f64>(window.window.scale_factor());
    let height = menu.len() as f64 * MENU_ITEM_HEIGHT + MENU_PADDING;
    let width = menu.iter().map(|item| item.label.chars().count()).max().unwrap_or(0) as f64 * MENU_CHAR_WIDTH + MENU_PADDING;
    let vertical = if y + height > size.height { format!("bottom: {}px;", size.height - y) } else { format!("top: {y}px;") };
    let horizontal = if x + width > size.width { format!("right: {}px;", size.width - x) } else { format!("left: {x}px;") };
    format!("{vertical} {horizontal}")
}

/// Times of day which make good x axis labels, in seconds from midnight, finest first.
const TICK_INTERVALS: [i64; 12] = [10, 30, 60, 300, 600, 900, 1800, 3600, 3 * 3600, 6 * 3600, 12 * 3600, 24 * 3600];

/// The roundest time of day from start to end, as unix timestamps, in the timezone, and its rank, which is higher the
/// rounder it is, e.g. midnight is the highest and a time on 10 seconds is 1, for the tick label of a bar.
pub fn time_tick(start: i64, end: i64, tz: &'static Tz) -> Option<(i64, u8)> {
    let offset = DateTime::from_unix_timestamp(start).ok()?.to_timezone(tz).offset().whole_seconds() as i64;
    let local_start = start + offset;
    TICK_INTERVALS.iter()
        .enumerate()
        .rev()
        .map(|(index, interval)| (local_start + (interval - local_start.rem_euclid(*interval)).rem_euclid(*interval) - offset, index as u8 + 1))
        .find(|(boundary, _)| *boundary < end)
}

/// The tick label and rank of a bar from start to end, its roundest time of day, with seconds if it is not on a minute.
pub fn time_tick_label(start: i64, end: i64, tz: &'static Tz) -> (Option<String>, u8) {
    let Some((boundary, rank)) = time_tick(start, end, tz) else {
        return (None, 0);
    };
    let format = if boundary.rem_euclid(60) == 0 { "[hour]:[minute]" } else { "[hour]:[minute]:[second]" };
    let label = DateTime::from_unix_timestamp(boundary).ok()
        .and_then(|time| time.to_timezone(tz).format(&time::format_description::parse(format).unwrap()).ok());
    (label, rank)
}

/// The x axis bands of tick labels, as (x, label, rank) from left to right, each from its x to the next one's, the last
/// to end_x, as (x, width, label), and whether all of the labels fit in their bands.
fn tick_bands(ticks: &[&(f64, String, u8)], end_x: f64) -> (Vec<(f64, f64, Option<String>)>, bool) {
    let bands: Vec<(f64, f64, Option<String>)> = ticks.iter()
        .enumerate()
        .map(|(index, (tick_x, label, _))| {
            let right = ticks.get(index + 1).map(|next| next.0).unwrap_or(end_x);
            (*tick_x, (right - tick_x).max(0.0), Some(label.clone()))
        })
        .collect();
    let fit = bands.iter()
        .all(|(_, band_width, label)| label.as_ref().is_none_or(|label| label_width(label) <= *band_width));
    (bands, fit)
}

/// An estimate of the width of an x axis label with the space around it.
fn label_width(label: &str) -> f64 {
    label.chars().count() as f64 * LABEL_CHAR_WIDTH + LABEL_PADDING
}

/// A period along the x axis, e.g. a day, shown as a faint band under the graph with its label in the middle.
#[derive(Clone, PartialEq, Debug)]
pub struct XBand {
    /// Unix timestamps.
    pub start: i64,
    pub end: i64,
    /// Ways of labelling it, longest first, the longest which fits is shown, e.g. "Wed 14 Oct", "14 Oct", "14".
    pub labels: Vec<String>,
}

/// A band for each day from start to end, as unix timestamps, in the timezone, for the x axis of a graph whose bars
/// are not periods which are easy to label, e.g. charging sessions or readings over several days.
pub fn day_bands(start: i64, end: i64, tz: &'static Tz) -> Vec<XBand> {
    let long_format = time::format_description::parse("[weekday repr:short] [day] [month repr:short]").unwrap();
    let short_format = time::format_description::parse("[day] [month repr:short]").unwrap();
    let day_format = time::format_description::parse("[day padding:none]").unwrap();
    let Ok(first) = DateTime::from_unix_timestamp(start) else {
        return Vec::new();
    };
    let mut bands = Vec::new();
    let mut day = first.to_timezone(tz).to_date();
    loop {
        let (day_start, day_end) = (day.at_midnight(tz), day.next_day().at_midnight(tz));
        if day_start.unix_timestamp() >= end {
            break;
        }
        bands.push(XBand {
            start: day_start.unix_timestamp(),
            end: day_end.unix_timestamp(),
            labels: [&long_format, &short_format, &day_format].iter()
                .map(|format| day.format(format).unwrap_or_default())
                .collect(),
        });
        day = day.next_day();
    }
    bands
}

/// A readable step between y axis ticks of at least raw, e.g. 0.25 or 50, and the number of decimal places it needs.
pub(crate) fn nice_step(raw: f64) -> (f64, usize) {
    if !(raw > 0.0) {
        return (1.0, 0);
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * magnitude);
    let mut decimals = 0;
    while decimals < 6 && ((step * 10f64.powi(decimals as i32)).round() - step * 10f64.powi(decimals as i32)).abs() > 1e-9 {
        decimals += 1;
    }
    (step, decimals)
}

/// Format an amount in minor units (e.g. pence) in the major units of the currency, e.g. £1.23
pub(crate) fn format_money(minor_units: f64, currency: Currency) -> String {
    let decimals = currency.minor_units() as usize;
    let major = minor_units / 10f64.powi(decimals as i32);
    let sign = if major < 0.0 { "-" } else { "" };
    format!("{sign}{}{:.decimals$}", currency_symbol(currency), major.abs())
}

fn currency_symbol(currency: Currency) -> String {
    if currency == Currency::NONE { String::new() } else { currency.symbol() }
}

/// A y axis label for the value, just the number, with decimal places for units and in the major units of the currency
/// for costs. The units are shown once, see axis_unit.
pub(crate) fn axis_label(value: f64, decimals: usize, mode: StackedBarMode, currency: Currency) -> String {
    match mode {
        StackedBarMode::Units => format!("{value:.decimals$}"),
        StackedBarMode::Cost => {
            let decimals = currency.minor_units() as usize;
            format!("{:.decimals$}", value / 10f64.powi(decimals as i32))
        },
    }
}

/// The units of the y axis labels, shown once above them, followed by suffix, e.g. kwh/day or £/h
/// The suffix of the unit of values per a number of seconds, e.g. /h for 3600.
fn per_suffix(seconds: i64) -> String {
    match seconds {
        60 => String::from("/min"),
        3600 => String::from("/h"),
        86400 => String::from("/day"),
        _ if seconds % 86400 == 0 => format!("/{}days", seconds / 86400),
        _ if seconds % 3600 == 0 => format!("/{}h", seconds / 3600),
        _ if seconds % 60 == 0 => format!("/{}min", seconds / 60),
        _ => format!("/{seconds}s"),
    }
}

pub(crate) fn axis_unit(mode: StackedBarMode, currency: Currency, unit: &str, suffix: &str) -> String {
    match mode {
        StackedBarMode::Units => format!("{unit}{suffix}"),
        StackedBarMode::Cost => format!("{}{suffix}", currency_symbol(currency)),
    }
}

#[component]
pub fn StackedBarGraph(
    points: Vec<StackedBarPoint>,
    /// The series stacked in each bar, bottom first.
    series: Vec<StackedBarSeries>,
    /// Whether the values are units or costs, for formatting.
    mode: StackedBarMode,
    /// The currency of costs, which are in its minor units, e.g. pence
    currency: Currency,
    /// The unit of consumption, e.g. kwh
    unit: String,
    /// Draw a histogram, where bar heights are per hour so that the area of each bar is its total. Needed when the bars
    /// have different widths.
    #[props(default)] per_hour: bool,
    /// Draw a histogram per this many seconds, as for per_hour, e.g. 10 for kWh per 10 seconds.
    #[props(default)] per_seconds: Option<i64>,
    /// Whether the series are named above the graph, e.g. not when there is a series for each of many sessions, which
    /// the tooltip names.
    #[props(default = true)] legend: bool,
    /// Called with the start and end of a bar when it is double clicked, to show it in more detail.
    #[props(default)] on_drill: Option<EventHandler<(i64, i64)>>,
    /// Called with the start of the first bar and the end of the last when the mouse is dragged across more than one
    /// bar, to show just those bars.
    #[props(default)] on_zoom: Option<EventHandler<(i64, i64)>>,
    /// The menu shown when a bar is right clicked, instead of the webview's menu.
    #[props(default)] menu: Vec<BarMenuItem>,
    /// Whether the tooltip is for the segment the mouse is over, showing just that series and the bar's total, rather
    /// than every series in the bar, e.g. when there is a series for each of many sessions.
    #[props(default)] segment_tooltip: bool,
    /// Bands under the graph, e.g. for each day, in place of the x axis labels of the bars, for when the bars are not
    /// periods which are easy to label, e.g. charging sessions.
    #[props(default)] x_bands: Vec<XBand>,
    /// Lines over the bars, see StackedBarLine.
    #[props(default)] lines: Vec<StackedBarLine>,
) -> Element {
    let mut hovered = use_signal::<Option<usize>>(|| None);
    // The series of the segment the mouse is over, with segment_tooltip.
    let mut hovered_series = use_signal::<Option<usize>>(|| None);
    // The bars where a drag started and where it is now, while the mouse button is down.
    let mut dragged = use_signal::<Option<(usize, usize)>>(|| None);
    // The bar which was right clicked and where, while its menu is open.
    let mut menu_at = use_signal::<Option<(BarClick, f64, f64)>>(|| None);
    let mut dimensions = use_signal(|| None::<(f64, f64)>);

    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return rsx! { p { "No data for this period" } };
    };

    let (width, height) = dimensions().unwrap_or((960.0, 320.0));
    let t_min = first.start as f64;
    let t_span = ((last.end - first.start) as f64).max(1.0);
    let plot_width = (width - LEFT - RIGHT).max(1.0);
    let plot_height = (height - TOP - BOTTOM).max(1.0);
    let x = |t: i64| LEFT + (t as f64 - t_min) / t_span * plot_width;
    // The label and colour of each segment, matching StackedBarPoint::values.
    let segment_names: Vec<(String, String)> = series.iter()
        .map(|series| (series.label.clone(), series.color.clone()))
        .collect();
    // The factor from a bar's values to its height, for a histogram per this many seconds, if it is one.
    let per = per_seconds.or(per_hour.then_some(3600));
    let scale = move |point: &StackedBarPoint| {
        match per {
            Some(per) => per as f64 / (point.end - point.start).max(1) as f64,
            None => 1.0,
        }
    };
    // The net total of each bar, for its tooltip.
    let totals: Vec<f64> = points
        .iter()
        .map(|point| point.values.iter().sum())
        .collect();
    // Positive values are stacked up from zero and negative ones down from it, e.g. energy exported, so the y axis goes
    // from the lowest stack of negative values, if there are any, to the highest of positive ones.
    let stack = |point: &StackedBarPoint, positive: bool| point.values.iter()
        .filter(|value| (**value > 0.0) == positive)
        .sum::<f64>() * scale(point);
    // Each line's value for each point, as drawn, see scale.
    let line_values: Vec<Vec<Option<f64>>> = lines.iter()
        .map(|line| points.iter().enumerate()
            .map(|(index, point)| line.values.get(index).copied().flatten().map(|value| value * scale(point)))
            .collect())
        .collect();
    let max_value = points.iter().map(|point| stack(point, true)).chain(line_values.iter().flatten().flatten().copied()).fold(0.0, f64::max);
    let min_value = points.iter().map(|point| stack(point, false)).chain(line_values.iter().flatten().flatten().copied()).fold(0.0, f64::min);
    let format_value = |value: f64| match mode {
        StackedBarMode::Units => format!("{value:.3} {unit}"),
        StackedBarMode::Cost => format_money(value, currency),
    };
    let (step, decimals) = nice_step((max_value - min_value) / Y_TICKS);
    let y_min = (min_value / step).floor() * step;
    let y_max = ((max_value / step).ceil() * step).max(y_min + step);
    let y = |value: f64| TOP + plot_height - (value - y_min) / (y_max - y_min) * plot_height;

    // Each segment of each bar, with the bar and series it is of, and its colour and position.
    let bars: Vec<(usize, usize, String, f64, f64, f64, f64)> = points
        .iter()
        .enumerate()
        .flat_map(|(bar, point)| {
            let slot_width = x(point.end) - x(point.start);
            let gap = if slot_width > 3.0 * BAR_GAP { BAR_GAP } else { 0.0 };
            let bar_x = x(point.start) + gap / 2.0;
            let bar_width = (slot_width - gap).max(0.0);
            let bar_scale = scale(point);
            // The ends of the stacks of positive and negative values so far.
            let (mut above, mut below) = (0.0, 0.0);

            point
                .values
                .iter()
                .copied()
                .zip(segment_names.iter())
                .enumerate()
                // Nothing is drawn for an empty segment, e.g. a series which is not in this bar.
                .filter(|(_, (value, _))| *value != 0.0)
                .map(move |(series, (value, (_, color)))| {
                    let value = value * bar_scale;
                    let (from, to) = if value > 0.0 {
                        above += value;
                        (above - value, above)
                    }
                    else {
                        below += value;
                        (below, below - value)
                    };
                    (bar, series, color.clone(), bar_x, y(to), bar_width, y(from) - y(to))
                })
        })
        .collect();

    // Each line, a step across each bar it has a value for, joined to the next if that has one too, with its colour and
    // whether it is dashed, leaving out those with nothing to draw.
    let line_paths: Vec<(String, String, bool)> = lines.iter().zip(&line_values)
        .map(|(line, values)| {
            let mut path = String::new();
            let mut joined = false;
            for (point, value) in points.iter().zip(values) {
                match value {
                    Some(value) => {
                        let line_y = y(*value);
                        path += &format!("{}{:.1} {line_y:.1} L{:.1} {line_y:.1} ", if joined { "L" } else { "M" }, x(point.start), x(point.end));
                        joined = true;
                    },
                    None => joined = false,
                }
            }
            (path, line.color.clone(), line.dashed)
        })
        .filter(|(path, _, _)| !path.is_empty())
        .collect();

    let y_ticks: Vec<(f64, String)> = (0..)
        // From a whole number of steps, so that zero is not a rounding error from it, e.g. -0.0.
        .map(|i| ((y_min / step).round() + i as f64) * step + 0.0)
        .take_while(|value| *value <= y_max + step / 2.0)
        .map(|value| {
            (y(value), axis_label(value, decimals, mode, currency))
        })
        .collect();
    let y_unit = axis_unit(mode, currency, &unit, &per.map(per_suffix).unwrap_or_default());

    // The bands under the x axis, each with its label in the middle: the x_bands, or if there are none, a band from each
    // labelled bar to the next one shown. As many tick labels are shown as fit: those of the lowest rank which still
    // fit with all the higher ranked ones, or if even the highest ranked ones do not fit, every so many of them, as
    // few apart as fit.
    let bands: Vec<(f64, f64, Option<String>)> = if x_bands.is_empty() {
        let all_x_ticks: Vec<(f64, String, u8)> = points.iter()
            .filter_map(|point| point.tick_label.as_ref().map(|label| (x(point.start), label.clone(), point.tick_rank)))
            .collect();
        let end_x = points.last().map(|point| x(point.end)).unwrap_or(LEFT);
        let ranks: BTreeSet<u8> = all_x_ticks.iter().map(|tick| tick.2).collect();
        let ranked = |rank: u8| all_x_ticks.iter().filter(|tick| tick.2 >= rank).collect::<Vec<_>>();
        ranks.iter()
            .map(|rank| tick_bands(&ranked(*rank), end_x))
            .find(|(_, fit)| *fit)
            .or_else(|| {
                let top = ranked(ranks.last().copied().unwrap_or_default());
                (1..=top.len())
                    .map(|every| tick_bands(&top.iter().step_by(every).copied().collect::<Vec<_>>(), end_x))
                    .find(|(_, fit)| *fit)
            })
            .map(|(bands, _)| bands)
            .unwrap_or_default()
    }
    else {
        x_bands.iter()
            .map(|band| {
                let left = x(band.start).max(LEFT);
                let right = x(band.end).min(width - RIGHT);
                let band_width = (right - left).max(0.0);
                // The longest label which fits, if any.
                let label = band.labels.iter().find(|label| label_width(label) <= band_width).cloned();
                (left, band_width, label)
            })
            .filter(|(_, band_width, _)| *band_width > 0.0)
            .collect()
    };

    // The legend, above the graph, as HTML, so that it wraps onto more lines if the labels need them: the series', then
    // the lines', each as a short line, dashed or not as it is.
    let legend_entries: Vec<(String, String)> = if legend { segment_names.clone() } else { Vec::new() };
    let line_legend: Vec<(String, String, bool)> = lines.iter().zip(&line_values)
        .filter(|(_, values)| legend && values.iter().any(Option::is_some))
        .map(|(line, _)| (line.label.clone(), line.color.clone(), line.dashed))
        .collect();

    // A column over each bar, the full height of the graph, which shows its tooltip when the mouse is over it and can
    // be double clicked or dragged across, and the start and end of each bar.
    let columns: Vec<(usize, f64, f64)> = points.iter()
        .enumerate()
        .map(|(index, point)| (index, x(point.start), (x(point.end) - x(point.start)).max(0.0)))
        .collect();
    let periods: std::rc::Rc<Vec<(i64, i64)>> = std::rc::Rc::new(points.iter().map(|point| (point.start, point.end)).collect());
    // The bars being dragged across, from the left of the first to the right of the last.
    let selection = dragged().map(|(from, to)| {
        let (first, last) = (from.min(to), from.max(to));
        let left = x(points[first].start);
        (left, x(points[last].end) - left)
    });

    let tooltip = hovered().and_then(|index| {
        points.get(index).map(|point| {
            let bar_x = x(point.start + (point.end - point.start) / 2);
            let tooltip_x = if bar_x > width / 2.0 {
                bar_x - TOOLTIP_WIDTH - 10.0
            } else {
                bar_x + 10.0
            }
            .clamp(LEFT, (width - TOOLTIP_WIDTH - RIGHT).max(LEFT));
            // The label, colour and value of each segment, leaving out those which are zero, or with segment_tooltip just
            // the one the mouse is over, if it is over one.
            let mut rows: Vec<(String, String, String)> = point.values
                .iter()
                .copied()
                .zip(&segment_names)
                .enumerate()
                .filter(|(series, (value, _))| *value != 0.0 && (!segment_tooltip || hovered_series() == Some(*series)))
                .map(|(_, (value, (label, color)))| (label.clone(), color.clone(), format_value(value)))
                .collect();
            for line in &lines {
                if let Some(value) = line.values.get(index).copied().flatten() {
                    rows.push((line.label.clone(), line.color.clone(), format_value(value)));
                }
            }
            // The rows start lower if there is a line of detail under the period.
            let top = TOP + if point.detail.is_some() { 14.0 } else { 0.0 };
            (tooltip_x, point.period_label.clone(), point.detail.clone(), top, rows, format_value(totals[index]))
        })
    });

    // The mouse handlers of a bar, for its column, and with segment_tooltip for its segments too, which are over the
    // column so that the mouse can be over them. series is the series of the segment, None for the column.
    let on_enter = move |index: usize, series: Option<usize>| move |_: MouseEvent| {
        hovered.set(Some(index));
        hovered_series.set(series);
        if let Some((from, _)) = dragged() {
            dragged.set(Some((from, index)));
        }
    };
    let on_down = move |index: usize| move |event: MouseEvent| {
        // Only the left button drags, the right one opens the menu, and only when there is something to zoom to.
        if on_zoom.is_some() && event.trigger_button() == Some(MouseButton::Primary) {
            dragged.set(Some((index, index)));
        }
    };
    let on_context_menu = {
        let periods = periods.clone();
        let has_menu = !menu.is_empty();
        move |index: usize| {
            let periods = periods.clone();
            move |event: MouseEvent| {
                event.prevent_default();
                if has_menu {
                    let (start, end) = periods[index];
                    let previous_start = if index > 0 { periods[index - 1].0 } else { start - (end - start) };
                    let point = event.client_coordinates();
                    menu_at.set(Some((BarClick { start, end, previous_start }, point.x, point.y)));
                }
            }
        }
    };
    let on_up = {
        let periods = periods.clone();
        move || {
            let periods = periods.clone();
            move |_: MouseEvent| {
                // A drag across more than one bar zooms to them, a click on one bar does nothing.
                if let (Some((from, to)), Some(on_zoom)) = (dragged.take(), on_zoom) {
                    if from != to {
                        on_zoom.call((periods[from.min(to)].0, periods[from.max(to)].1));
                    }
                }
            }
        }
    };
    let on_double_click = {
        let periods = periods.clone();
        move |index: usize| {
            let periods = periods.clone();
            move |_: MouseEvent| {
                if let Some(on_drill) = on_drill {
                    on_drill.call(periods[index]);
                }
            }
        }
    };

    rsx! {
        if !legend_entries.is_empty() || !line_legend.is_empty() {
            div { class: "stacked-legend",
                for (label, color) in legend_entries {
                    span { class: "stacked-legend-item",
                        span { class: "stacked-legend-key", style: "background: {color};" }
                        "{label}"
                    }
                }
                for (label, color, dashed) in line_legend {
                    span { class: "stacked-legend-item",
                        span { class: if dashed { "stacked-legend-line" } else { "stacked-legend-line solid" }, style: "border-top-color: {color};" }
                        "{label}"
                    }
                }
            }
        }
        // The frame is measured, not the svg, because some versions of WebKit, e.g. on an Intel Mac, give the size of
        // what is drawn in an svg as its size, which is a little smaller than the svg, so each resize shrank the graph.
        div { class: "graph-frame",
            onresize: move |event| {
                match event.data().get_content_box_size() {
                    Ok(size) if size.width > 0.0 && size.height > 0.0 => {
                        dimensions.set(Some((size.width, size.height)));
                    }
                    Ok(_) => eprintln!("Stacked consumption graph resized to an empty size"),
                    Err(error) => eprintln!("Failed to read stacked consumption graph size: {error:?}"),
                }
            },
            svg {
                class: format_args!(
                    "stacked-consumption-graph{}{}",
                    if dimensions().is_some() { " ready" } else { "" },
                    if dragged().is_some_and(|(from, to)| from != to) { " zooming" } else { "" },
                ),
                view_box: "0 0 {width} {height}",
                role: "img",
                onmouseleave: move |_| {
                    hovered.set(None);
                    hovered_series.set(None);
                    dragged.set(None);
                },
                // Each tick is a keyed group, as unkeyed items of more than one element can leave old labels behind when the
                // number of ticks changes.
                for (index, (ty, label)) in y_ticks.into_iter().enumerate() {
                    g { key: "{index}",
                        line { class: "stacked-grid", x1: LEFT, x2: width - RIGHT, y1: ty, y2: ty }
                        text { class: "stacked-axis-label", x: LEFT - 8.0, y: ty, text_anchor: "end", dominant_baseline: "middle", "{label}" }
                    }
                }
                // The units of the y axis labels, above them in the row of the legend.
                // From the left edge, as with a period, e.g. kWh/10s, it can be wider than the labels under it.
                text { class: "stacked-axis-label", x: 4.0, y: 24.0, text_anchor: "start", "{y_unit}" }
                // A gap of a pixel each side shows where one band ends and the next starts.
                for (index, (band_x, band_width, label)) in bands.into_iter().enumerate() {
                    g { key: "{index}",
                        rect { class: "stacked-x-band", x: band_x + 1.0, y: TOP + plot_height + 4.0, width: (band_width - 2.0).max(0.0), height: BOTTOM - 6.0 }
                        if let Some(label) = label {
                            text { class: "stacked-axis-label", x: band_x + band_width / 2.0, y: height - 10.0, text_anchor: "middle", "{label}" }
                        }
                    }
                }
                if !segment_tooltip {
                    for (_, _, color, bx, by, bw, bh) in bars.iter().cloned() {
                        rect {
                            class: "stacked-bar",
                            fill: "{color}",
                            x: bx,
                            y: by,
                            width: bw,
                            height: bh,
                        }
                    }
                }
                for (line_path, color, dashed) in line_paths {
                    path { class: if dashed { "stacked-line" } else { "stacked-line solid" }, stroke: "{color}", d: "{line_path}" }
                }
                for (index, column_x, column_width) in columns {
                    rect {
                        class: "stacked-hit",
                        x: column_x,
                        y: TOP,
                        width: column_width,
                        height: plot_height,
                        onmouseenter: on_enter(index, None),
                        onmousedown: on_down(index),
                        oncontextmenu: on_context_menu(index),
                        onmouseup: on_up(),
                        ondoubleclick: on_double_click(index),
                    }
                }
                // Over the columns, so that the tooltip can be for the segment the mouse is over.
                if segment_tooltip {
                    for (index, series, color, bx, by, bw, bh) in bars.iter().cloned() {
                        rect {
                            class: "stacked-bar",
                            fill: "{color}",
                            x: bx,
                            y: by,
                            width: bw,
                            height: bh,
                            onmouseenter: on_enter(index, Some(series)),
                            onmousedown: on_down(index),
                            oncontextmenu: on_context_menu(index),
                            onmouseup: on_up(),
                            ondoubleclick: on_double_click(index),
                        }
                    }
                }
                if let Some((selection_x, selection_width)) = selection {
                    rect { class: "stacked-selection", x: selection_x, y: TOP, width: selection_width, height: plot_height }
                }

                if let Some((tx, period, detail, top, rows, total)) = tooltip {
                    g { class: "stacked-tooltip",
                        rect { class: "stacked-tooltip-background", x: tx, y: TOP, width: TOOLTIP_WIDTH, height: top - TOP + 42.0 + rows.len() as f64 * 14.0, rx: 4 }
                        text { class: "stacked-tooltip-period", x: tx + TOOLTIP_TEXT_X, y: TOP + 16.0, "{period}" }
                        if let Some(detail) = detail {
                            text { class: "stacked-tooltip-value", x: tx + TOOLTIP_TEXT_X, y: TOP + 30.0, "{detail}" }
                        }
                        text { class: "stacked-tooltip-value", x: tx + TOOLTIP_TEXT_X, y: top + 34.0, "Total: {total}" }
                        for (index, (label, color, value)) in rows.iter().enumerate() {
                            rect { class: "stacked-legend-swatch", fill: "{color}", x: tx + 8.0, y: top + 43.0 + index as f64 * 14.0, width: 10.0, height: 10.0 }
                            text { class: "stacked-tooltip-value", x: tx + TOOLTIP_TEXT_X, y: top + 52.0 + index as f64 * 14.0, "{label}: {value}" }
                        }
                    }

                }
            }
        }
        if let Some((click, menu_x, menu_y)) = menu_at() {
            // Covers the page so that clicking anywhere else, or right clicking, closes the menu.
            div {
                class: "menu-backdrop",
                onclick: move |_| menu_at.set(None),
                oncontextmenu: move |event| {
                    event.prevent_default();
                    menu_at.set(None);
                },
            }
            div {
                class: "dropdown-menu bar-menu",
                style: menu_position(menu_x, menu_y, &menu),
                for item in menu.iter().cloned() {
                    button {
                        r#type: "button",
                        disabled: !item.enabled,
                        onclick: move |_| {
                            menu_at.set(None);
                            item.action.call(click);
                        },
                        "{item.label}"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use sparko_graphql::types::Currency;

    use super::{StackedBarMode, axis_label, axis_unit, day_bands, format_money, nice_step, time_tick_label};

    fn time(text: &str) -> i64 {
        time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap().unix_timestamp()
    }

    #[test]
    fn a_bar_is_labelled_with_its_roundest_local_time() {
        let london = time_tz::timezones::db::europe::LONDON;
        // 10 second bars in BST, one with a 3 hour boundary in it but not at its start, as when a reading is missing.
        assert_eq!(time_tick_label(time("2026-10-24T14:00:10Z"), time("2026-10-24T14:00:20Z"), london), (Some(String::from("15:00:10")), 1));
        assert_eq!(time_tick_label(time("2026-10-24T13:59:55Z"), time("2026-10-24T14:00:15Z"), london), (Some(String::from("15:00")), 9));
        // Half hour bars, local midnight is the roundest.
        let (_, half_past) = time_tick_label(time("2026-10-24T13:30:00Z"), time("2026-10-24T14:00:00Z"), london);
        let (_, hour) = time_tick_label(time("2026-10-24T14:00:00Z"), time("2026-10-24T14:30:00Z"), london);
        let (midnight, day) = time_tick_label(time("2026-10-24T23:00:00Z"), time("2026-10-24T23:30:00Z"), london);
        assert!(half_past < hour && hour < day);
        assert_eq!(midnight.as_deref(), Some("00:00"));
    }

    #[test]
    fn there_is_a_band_for_each_local_day_including_the_one_with_the_clock_change() {
        let london = time_tz::timezones::db::europe::LONDON;
        // From 20:00 BST on Saturday 24 October to 08:00 GMT on Monday 26 October, across the clocks going back.
        let bands = day_bands(time("2026-10-24T19:00:00Z"), time("2026-10-26T08:00:00Z"), london);
        let labels: Vec<&str> = bands.iter().map(|band| band.labels[0].as_str()).collect();
        assert_eq!(labels, ["Sat 24 Oct", "Sun 25 Oct", "Mon 26 Oct"]);
        assert_eq!(bands[1].labels, ["Sun 25 Oct", "25 Oct", "25"]);
        // Sunday is 25 hours long.
        assert_eq!(bands[1].end - bands[1].start, 25 * 3600);
    }

    #[test]
    fn nice_step_rounds_up_to_readable_values() {
        assert_eq!(nice_step(0.0), (1.0, 0));
        assert_eq!(nice_step(0.3), (0.5, 1));
        assert_eq!(nice_step(0.21), (0.25, 2));
        assert_eq!(nice_step(1.7), (2.0, 0));
        assert_eq!(nice_step(42.0), (50.0, 0));
    }

    #[test]
    fn money_is_formatted_in_major_units_with_the_currency_symbol() {
        let gbp = Currency::from_code("GBP").unwrap();
        assert_eq!(format_money(123.0, gbp), "£1.23");
        assert_eq!(format_money(-5.5, gbp), "-£0.06");
        assert_eq!(format_money(1234.0, Currency::from_code("JPY").unwrap()), "¥1234");
        assert_eq!(format_money(12.0, Currency::NONE), "12");
    }

    #[test]
    fn axis_labels_are_just_numbers_and_the_units_are_shown_once() {
        let gbp = Currency::from_code("GBP").unwrap();
        assert_eq!(axis_label(2.5, 1, StackedBarMode::Units, gbp), "2.5");
        assert_eq!(axis_label(125.0, 0, StackedBarMode::Cost, gbp), "1.25");
        assert_eq!(axis_unit(StackedBarMode::Units, gbp, "kwh", "/day"), "kwh/day");
        assert_eq!(axis_unit(StackedBarMode::Cost, gbp, "kwh", "/h"), "£/h");
        assert_eq!(axis_unit(StackedBarMode::Cost, Currency::NONE, "kwh", ""), "");
    }
}
