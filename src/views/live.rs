//! Home's Live page: the site's topology, see power::topology, drawn as on the Site page, with what each circuit is
//! taking or giving now, see power::live, and dots moving along the lines between them in the direction the energy
//! flows, faster the more there is, as the NSPanel's dashboard draws the grid, the house, the solar panels, the
//! batteries and the car. While it is shown, it has the Tesla module read the capture devices every REFRESH, a few
//! seconds after their polls, see POLLED, rather than every few minutes, and is worked out again after each read, so it
//! is about as fresh as their polls; a circuit worked out from older readings, e.g. the zappi's minutes, which come from
//! myenergi every few minutes, says when they are of.

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use dioxus::prelude::*;

use super::site::{BOX_WIDTH, MARGIN, ROW_GAP, Placed, box_height, category_colours, column_x, place, use_colour};
use crate::components::error_text::ErrorText;
use crate::power::live::{Live, live};
use crate::power::topology::{Circuit, Use};
use crate::profile::Settings as ProfileSettings;
use crate::{MarcoSparkoContext, ModuleRegistrations};

/// How often the circuits' power is worked out again, and the capture devices read, as often as they poll the gateway.
const REFRESH: i64 = 10;
/// How long after each REFRESH, from the start of the minute, the devices are read, so that they have saved their poll,
/// and the power worked out, so that the read is done.
const POLLED: i64 = 4;
const READ: i64 = 3;
/// How far back the measurements are loaded, enough for the latest every source has, see power::live.
const LOADED: i64 = 15 * 60;
/// Less power than this, in W, has no dots moving, so that a few W either way, e.g. a site meter the Powerwalls hold
/// at about none, does not look like a flow. Its figure is still shown, so that they all add up; solar panels' noise
/// is none already, see power::live::SOLAR_NOISE.
const STILL: f64 = 10.0;

/// Whether a circuit's power is too little for dots, see STILL.
fn still(watts: f64) -> bool {
    watts.abs() < STILL
}

/// The circuits' power, of what the profile's modules have measured, worked out again every REFRESH.
#[component]
pub fn LivePage() -> Element {
    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let registrations = use_context::<ModuleRegistrations>();
    let settings = use_context::<Signal<ProfileSettings>>();
    let found = use_resource(move || {
        let context = context_signal.read().clone();
        let registrations = registrations.clone();
        async move {
            let found = super::electricity::find(context.ok_or_else(|| anyhow!("There is no profile"))?, registrations).await?;
            anyhow::Ok(Arc::new(found))
        }
    });
    let mut tick = use_signal(|| 0u32);
    let statuses = try_use_context::<crate::polling::PollStatuses>();
    use_future(move || async move {
        loop {
            // POLLED seconds after the next multiple of REFRESH, as the devices poll on them.
            let now = time::OffsetDateTime::now_utc();
            let next = (now.unix_timestamp() - POLLED).div_euclid(REFRESH) * REFRESH + REFRESH + POLLED;
            let wait = (next as f64 - (now.unix_timestamp() as f64 + now.nanosecond() as f64 / 1e9)).max(0.0);
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
            if let Some(statuses) = statuses {
                crate::polling::poll_now(statuses, "tesla");
            }
            tokio::time::sleep(Duration::from_secs(READ as u64)).await;
            tick += 1;
        }
    });
    let topology = settings.read().topology.clone();
    let live_power = use_resource(move || {
        let _ = tick();
        let found = match &*found.read() {
            Some(Ok(found)) => Some(found.clone()),
            _ => None,
        };
        let topology = settings.read().topology.clone();
        async move {
            let (Some(found), Some(topology)) = (found, topology) else { return anyhow::Ok(None) };
            let now = crate::clock::now_utc().unix_timestamp();
            let mut measurements = Vec::new();
            for source in &found.measurements {
                measurements.push(source.load(now - LOADED, now).await?);
            }
            anyhow::Ok(live(&topology, &measurements))
        }
    });

    let Some(topology) = topology else {
        return rsx! {
            h1 { "Live" }
            p { "Set the site's topology on the Site page, to see what each circuit is taking or giving now." }
        };
    };
    let colours = category_colours(&settings.read());
    rsx! {
        h1 { "Live" }
        match &*live_power.read() {
            None => rsx! { p { "Loading..." } },
            Some(Err(error)) => rsx! { div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the measurements" } } },
            Some(Ok(None)) => rsx! { p { "None of the devices placed in the topology has measured anything in the last 15 minutes." } },
            Some(Ok(Some(live))) => rsx! {
                p { class: "timezone", {as_of(live.time)} }
                div { class: "topology-scroll", LiveDiagram { topology: topology.clone(), live: live.clone(), colours } }
            },
        }
    }
}

/// A time by the computer's clock, e.g. 15:58:30.
fn clock_time(time: i64) -> String {
    use time_tz::OffsetDateTimeExt;
    let tz = time_tz::system::get_timezone().unwrap_or(time_tz::timezones::db::UTC);
    let format = time::macros::format_description!("[hour]:[minute]:[second]");
    time::OffsetDateTime::from_unix_timestamp(time).map(|time| time.to_timezone(tz).format(&format).unwrap_or_default()).unwrap_or_default()
}

/// When the power is of, and how long ago, e.g. As of 15:58:30, 2 minutes ago.
fn as_of(time: i64) -> String {
    let shown = clock_time(time);
    let ago = crate::clock::now_utc().unix_timestamp() - time;
    let ago = match ago {
        ..=59 => String::from("under a minute ago"),
        60..=119 => String::from("a minute ago"),
        _ => format!("{} minutes ago", ago / 60),
    };
    format!("As of {shown}, {ago}")
}

/// A circuit's power, in whole W, so that the circuits' add up, to within the rounding, e.g. 1940 W, and what it is
/// doing, e.g. charging, for a battery.
fn describe(use_: &Use, top: bool, watts: Option<f64>) -> String {
    let Some(watts) = watts else { return String::from("not known") };
    let shown = format!("{:.0} W", watts.abs());
    if watts.round() == 0.0 {
        return String::from("0 W");
    }
    match (use_, top, watts > 0.0) {
        (_, true, true) => format!("importing {shown}"),
        (_, true, false) => format!("exporting {shown}"),
        (Use::Battery, _, true) => format!("charging {shown}"),
        (Use::Battery, _, false) => format!("discharging {shown}"),
        (Use::Solar, _, _) => format!("making {shown}"),
        (_, _, true) => shown,
        (_, _, false) => format!("giving {shown}"),
    }
}

/// The topology as the Site page draws it, each circuit with its power, and the lines between them with dots moving
/// from where the energy comes to where it goes, the faster the more power, see .topology-line.flow in main.css.
#[component]
fn LiveDiagram(topology: Circuit, live: Live, colours: std::collections::BTreeMap<String, String>) -> Element {
    // Laid out as the Site page's, with two lines in place of the channels: the power, and when it is of, if it is
    // older than the rest.
    let layout = with_lines(&topology);
    let box_height = box_height(LINES);
    let mut placed: Vec<Placed> = Vec::new();
    let mut top = MARGIN;
    place(&layout, 0, None, &mut top, &mut placed);
    let depth = placed.iter().map(|placed| placed.depth).max().unwrap_or(0);
    let width = column_x(depth) + BOX_WIDTH + MARGIN;
    let height = top - ROW_GAP + MARGIN;
    let placed_y: Vec<f64> = placed.iter().map(|placed| placed.y).collect();
    let power = |index: usize| live.power.get(index).copied().flatten();
    rsx! {
        svg {
            class: "topology-diagram live-diagram",
            view_box: "0 0 {width} {height}",
            width: "{width}",
            role: "img",
            for (index, placed) in placed.iter().enumerate() {
                if let Some(parent) = placed.parent {
                    {
                        let parent_y = placed_y[parent];
                        let (from, to) = (column_x(placed.depth - 1) + BOX_WIDTH, column_x(placed.depth));
                        let middle = (from + to) / 2.0;
                        let path = format!("M {from} {parent_y} H {middle} V {} H {to}", placed.y);
                        // Into the circuit is along the line, from what feeds it; out of it, e.g. solar, back along it.
                        let watts = power(index).unwrap_or_default();
                        let moving = !still(watts);
                        let class = match (moving, watts > 0.0) {
                            (false, _) => "topology-line",
                            (true, true) => "topology-line flow",
                            (true, false) => "topology-line flow reverse",
                        };
                        // A second for a dot to go from one to the next at 4 kW, slower for less.
                        let seconds = (4.0 / (watts.abs() / 1000.0).max(0.1)).clamp(0.3, 20.0) / 4.0;
                        rsx! {
                            path { key: "line{index}", class: "topology-line", d: "{path}" }
                            if moving {
                                path {
                                    key: "flow{index}",
                                    class,
                                    d: "{path}",
                                    stroke: use_colour(&placed.use_, &colours),
                                    style: "animation-duration: {seconds:.2}s",
                                }
                            }
                        }
                    }
                }
            }
            for (index, placed) in placed.iter().enumerate() {
                g { key: "box{index}",
                    rect {
                        class: "topology-box",
                        x: column_x(placed.depth),
                        y: placed.y - box_height / 2.0,
                        width: BOX_WIDTH,
                        height: box_height,
                        rx: 8.0,
                        stroke: use_colour(&placed.use_, &colours),
                    }
                    text {
                        class: "topology-name",
                        x: column_x(placed.depth) + 12.0,
                        y: placed.y - box_height / 2.0 + 21.0,
                        fill: use_colour(&placed.use_, &colours),
                        "{placed.name}"
                    }
                    text {
                        class: "live-power",
                        x: column_x(placed.depth) + 12.0,
                        y: placed.y - box_height / 2.0 + 41.0,
                        {describe(&placed.use_, placed.parent.is_none(), power(index))}
                    }
                    if let Some(older) = live.older.get(index).copied().flatten() {
                        text {
                            class: "live-older",
                            x: column_x(placed.depth) + 12.0,
                            y: placed.y - box_height / 2.0 + 57.0,
                            {format!("as of {}", clock_time(older))}
                        }
                    }
                }
            }
        }
    }
}

/// How many lines a circuit's box has under its name, see with_lines.
const LINES: usize = 2;

/// The topology with LINES lines for each circuit in place of its channels, which the layout gives each a line of, see
/// site::place, so that every box is as tall.
fn with_lines(circuit: &Circuit) -> Circuit {
    Circuit {
        channels: vec![String::new(); LINES],
        children: circuit.children.iter().map(with_lines).collect(),
        ..circuit.clone()
    }
}
