//! The Temp & Humidity page of Home: the latest reading of each capture device which measures them, and a graph of
//! each series over the range chosen, a line for each device, see climate.rs.

use std::sync::Arc;

use dioxus::prelude::*;
use time::OffsetDateTime;
use time_tz::{OffsetDateTimeExt, PrimitiveDateTimeExt, Tz};

use crate::MarcoSparkoContext;
use crate::climate::{self, DeviceClimate, SERIES, Series};
use crate::components::date_range_picker::{DateRangePicker, RangeSelection};
use crate::components::error_text::ErrorText;
use crate::polling::PollStatuses;

/// The graph's size, in the SVG's units, and its margins, for the axes' labels.
const WIDTH: f64 = 900.0;
const HEIGHT: f64 = 240.0;
const LEFT: f64 = 56.0;
const RIGHT: f64 = 16.0;
const TOP: f64 = 12.0;
const BOTTOM: f64 = 28.0;

/// Each device's line's colour, in turn.
const COLOURS: [&str; 6] = ["#4e9af1", "#f1a14e", "#6cc070", "#d46ad4", "#e05a5a", "#c8c86a"];

/// Periods further apart than this are not joined, as the device was not measuring between them, e.g. it restarted.
const GAP: i64 = 2 * 60;

/// The timezone times are shown in, the computer's.
fn timezone() -> &'static Tz {
    time_tz::system::get_timezone().unwrap_or(time_tz::timezones::db::europe::LONDON)
}

#[component]
pub fn ClimatePage() -> Element {
    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let profile_name = context_signal.read().as_ref().map(|context| context.profile.active_profile.name.clone()).unwrap_or_default();
    let statuses = use_context::<PollStatuses>();
    let tz = timezone();
    let now = crate::clock::now_utc();
    let range = use_signal(|| RangeSelection::day_containing(now.unix_timestamp(), tz).unwrap_or_else(|| RangeSelection::current_month(tz)));
    // Read so that the page shows what each read of the devices saved, see ClimateReader.
    let _read = statuses.read().get(climate::MODULE_ID).and_then(|status| status.last_time);

    let dir = match climate::store(&profile_name) {
        Ok(dir) => dir,
        Err(error) => return rsx! { ErrorText { error: format!("{error:#}") } },
    };
    let (from, to) = match range().to_date_times(tz) {
        Ok((from, to)) => (from.unix_timestamp(), to.unix_timestamp()),
        Err(error) => return rsx! { ErrorText { error } },
    };
    let at = |seconds: i64| OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(now);
    let devices = climate::load(&dir, at(from), at(to));
    // The latest of each, whatever the range.
    let latest = climate::load(&dir, now - time::Duration::days(1), now + time::Duration::minutes(1));

    rsx! {
        h1 { "Temperature and Humidity" }
        p { class: "note",
            "What the capture devices with a sensor of their own measure, e.g. the Waveshare RLCD, every 10 seconds, read from them every 5 minutes. Each device's are its own, a line each in the graphs."
        }
        if latest.is_empty() {
            p { "No capture device has measured the temperature and humidity in the last day. Enable Temperature and Humidity Log in a device's Settings on the Devices page." }
        } else {
            table { class: "display",
                tr {
                    th { "Device" }
                    for series in SERIES {
                        th { "{series.label}" }
                    }
                    th { "As of" }
                }
                for device in latest.iter() {
                    tr {
                        td { "{device.name}" }
                        for (series, periods) in SERIES.iter().zip(&device.series) {
                            td { class: "numeric", {periods.last().map(|period| shown(series, period.value)).unwrap_or_default()} }
                        }
                        td { {device.series.iter().filter_map(|periods| periods.last()).map(|period| period.end).max().map(|end| local_time(end, tz)).unwrap_or_default()} }
                    }
                }
            }
        }
        div { class: "date-range date-range-row",
            DateRangePicker { range, timezone: tz }
        }
        if devices.is_empty() {
            p { "No readings in this period." }
        } else {
            div { class: "climate-legend",
                for (index, device) in devices.iter().enumerate() {
                    span {
                        span { class: "climate-swatch", style: "background-color: {COLOURS[index % COLOURS.len()]}" }
                        "{device.name}"
                    }
                }
            }
            for (index, series) in SERIES.iter().enumerate() {
                h2 { "{series.label} ({series.unit})" }
                {graph(series, index, &devices, from, to, tz)}
            }
        }
    }
}

/// value as series shows it, e.g. 23.4 °C.
fn shown(series: &Series, value: f64) -> String {
    format!("{value:.*} {}", series.decimals, series.unit)
}

fn local_time(seconds: i64, tz: &Tz) -> String {
    let format = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]");
    OffsetDateTime::from_unix_timestamp(seconds).map(|time| time.to_timezone(tz).format(&format).unwrap_or_default()).unwrap_or_default()
}

/// The graph of the series at index of SERIES, of each device's periods, from from to to, as steps, a line each.
fn graph(series: &Series, index: usize, devices: &[DeviceClimate], from: i64, to: i64, tz: &Tz) -> Element {
    let lines: Vec<(&str, &[climate::Period])> = devices.iter().map(|device| (device.name.as_str(), device.series[index].as_slice())).collect();
    series_graph(series, &lines, from, to, tz)
}

/// The graph of a series, of the periods of each of lines, by name, from from to to, as steps, a line each, e.g. a
/// device's battery's voltage on the Devices page.
pub fn series_graph(series: &Series, lines: &[(&str, &[climate::Period])], from: i64, to: i64, tz: &Tz) -> Element {
    let values: Vec<f64> = lines.iter().flat_map(|(_, periods)| periods.iter().map(|period| period.value)).collect();
    if values.is_empty() {
        return rsx! { p { "No {series.label.to_lowercase()} readings in this period." } };
    }
    let (min, max) = values.iter().fold((f64::MAX, f64::MIN), |(min, max), value| (min.min(*value), max.max(*value)));
    // At least a degree, 5 %, or a tenth of a volt, so that the smallest steps do not fill the graph.
    let span = (max - min).max(match series.decimals { 0 => 5.0, 1 => 1.0, _ => 0.1 });
    let (step, decimals) = crate::components::stacked_bar_graph::nice_step(span / 4.0);
    let low = (min / step).floor() * step;
    let high = ((max / step).ceil() * step).max(low + step);
    let plot_width = WIDTH - LEFT - RIGHT;
    let plot_height = HEIGHT - TOP - BOTTOM;
    let x = move |time: i64| LEFT + (time.clamp(from, to) - from) as f64 / (to - from).max(1) as f64 * plot_width;
    let y = move |value: f64| TOP + (high - value) / (high - low) * plot_height;

    let y_ticks: Vec<(f64, String)> = (0..)
        .map(|tick| low + tick as f64 * step)
        .take_while(|value| *value <= high + step / 1e6)
        .map(|value| (y(value), format!("{value:.decimals$}")))
        .collect();
    let x_ticks: Vec<(f64, String)> = time_ticks(from, to, tz).into_iter().map(|(time, label)| (x(time), label)).collect();
    let lines: Vec<(String, &str, &str)> = lines.iter().enumerate()
        .map(|(line_index, (name, periods))| (step_path(periods, from, to, &x, &y), COLOURS[line_index % COLOURS.len()], *name))
        .collect();

    rsx! {
        svg { class: "climate-graph", view_box: "0 0 {WIDTH} {HEIGHT}",
            for (position, label) in y_ticks {
                line { class: "grid", x1: "{LEFT}", x2: "{WIDTH - RIGHT}", y1: "{position}", y2: "{position}" }
                text { x: "{LEFT - 6.0}", y: "{position + 4.0}", text_anchor: "end", "{label}" }
            }
            for (position, label) in x_ticks {
                line { class: "grid", x1: "{position}", x2: "{position}", y1: "{TOP}", y2: "{HEIGHT - BOTTOM}" }
                text { x: "{position}", y: "{HEIGHT - BOTTOM + 16.0}", text_anchor: "middle", "{label}" }
            }
            for (path, colour, name) in lines {
                path { d: "{path}", stroke: "{colour}", stroke_width: "2", fill: "none",
                    title { "{name}" }
                }
            }
        }
    }
}

/// The SVG path of periods, in order, as steps: along each period at its value, up or down to the next if it follows
/// on, otherwise a gap, see GAP.
fn step_path(periods: &[climate::Period], from: i64, to: i64, x: &dyn Fn(i64) -> f64, y: &dyn Fn(f64) -> f64) -> String {
    let mut path = String::new();
    let mut last_end = None;
    for period in periods.iter().filter(|period| period.end >= from && period.start <= to) {
        let (start, end, height) = (x(period.start), x(period.end), y(period.value));
        match last_end {
            Some(last) if period.start - last <= GAP => path.push_str(&format!(" V {height:.1} H {end:.1}")),
            _ => path.push_str(&format!(" M {start:.1} {height:.1} H {end:.1}")),
        }
        last_end = Some(period.end);
    }
    path
}

/// The times, from from to to, to mark on the time axis, with their labels: every few hours, at times of day, over a
/// day or two, otherwise at midnights, with their dates, no more than about 8 of them.
fn time_ticks(from: i64, to: i64, tz: &Tz) -> Vec<(i64, String)> {
    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;
    let spacing = [HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, DAY, 2 * DAY, 7 * DAY, 14 * DAY, 28 * DAY]
        .into_iter()
        .find(|spacing| (to - from) / spacing <= 8)
        .unwrap_or(28 * DAY);
    let hours = time::macros::format_description!("[hour]:[minute]");
    let date = time::macros::format_description!("[day padding:none] [month repr:short]");
    let Ok(start) = OffsetDateTime::from_unix_timestamp(from) else { return Vec::new() };
    let midnight = |day: time::Date| day.with_time(time::Time::MIDNIGHT).assume_timezone(tz).take_first().map(|time| time.unix_timestamp());
    let mut ticks = Vec::new();
    let mut day = start.to_timezone(tz).date();
    while let Some(day_start) = midnight(day) {
        if day_start > to {
            break;
        }
        // Within the day, by the clock, so that the ticks stay on the hours across a change of the clocks.
        let in_day = if spacing < DAY { (0..DAY / spacing).map(|tick| day_start + tick * spacing).collect() } else { vec![day_start] };
        for time in in_day.into_iter().filter(|time| (from..=to).contains(time)) {
            let local = OffsetDateTime::from_unix_timestamp(time).map(|time| time.to_timezone(tz));
            let label = match local {
                Ok(local) if spacing < DAY && local.time() != time::Time::MIDNIGHT => local.format(&hours).unwrap_or_default(),
                Ok(local) => local.format(&date).unwrap_or_default(),
                Err(_) => continue,
            };
            ticks.push((time, label));
        }
        let Some(next) = day.checked_add(time::Duration::days((spacing / DAY).max(1))) else { break };
        day = next;
    }
    ticks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_marked_every_3_hours_and_a_week_at_midnights() {
        let london = time_tz::timezones::db::europe::LONDON;
        let at = |text: &str| OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap().unix_timestamp();
        let day = time_ticks(at("2026-11-04T00:00:00Z"), at("2026-11-05T00:00:00Z"), london);
        assert_eq!(day.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>(),
            vec!["4 Nov", "03:00", "06:00", "09:00", "12:00", "15:00", "18:00", "21:00", "5 Nov"]);
        let week = time_ticks(at("2026-11-02T00:00:00Z"), at("2026-11-09T00:00:00Z"), london);
        assert_eq!(week.len(), 8);
        assert_eq!(week[0].1, "2 Nov");
    }

    #[test]
    fn periods_which_follow_on_are_joined_and_gaps_are_not() {
        let periods = [
            climate::Period { start: 0, end: 60, value: 1.0 },
            climate::Period { start: 60, end: 120, value: 2.0 },
            climate::Period { start: 1000, end: 1100, value: 2.0 },
        ];
        let path = step_path(&periods, 0, 1100, &|time| time as f64, &|value| value * 10.0);
        assert_eq!(path, " M 0.0 10.0 H 60.0 V 20.0 H 120.0 M 1000.0 20.0 H 1100.0");
    }
}
