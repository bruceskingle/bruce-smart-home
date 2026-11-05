//! Home's Site page: the site's topology, where in the house each module's channels measure, see power::topology, drawn
//! as a diagram from the grid to the house, and edited as a list of its circuits, which is saved with the profile's
//! settings. The Electricity page splits the grid's import by use with it.

use std::sync::Arc;

use anyhow::anyhow;
use dioxus::prelude::*;

use crate::components::error_text::ErrorText;
use crate::power::topology::{Channel, Circuit, Use};
use crate::profile::Settings as ProfileSettings;
use crate::{MarcoSparkoContext, ModuleRegistrations};

/// The size of a circuit's box in the diagram, with one channel, and the gaps between them, in the SVG's units. Each
/// channel is a line of its own, see box_height.
const BOX_WIDTH: f64 = 190.0;
const BOX_HEIGHT: f64 = 54.0;
const LINE_HEIGHT: f64 = 16.0;
const COLUMN_GAP: f64 = 70.0;
const ROW_GAP: f64 = 18.0;
const MARGIN: f64 = 8.0;

/// The colour of a use, as the Electricity page's labels have them, e.g. the car's is EV Low Rate's.
fn use_colour(use_: Use) -> &'static str {
    match use_ {
        Use::Grid => "#b0b0b0",
        Use::Board => "#e0e0e0",
        Use::Car => "#4f6bff",
        Use::Battery => "#9b51e0",
        Use::Solar => "#f2c94c",
        Use::House => "#46ee43",
    }
}

fn use_id(use_: Use) -> &'static str {
    match use_ {
        Use::Grid => "grid",
        Use::Board => "board",
        Use::Car => "car",
        Use::Battery => "battery",
        Use::Solar => "solar",
        Use::House => "house",
    }
}

/// The circuit at path, the indexes of the children down to it, from circuit.
fn at<'a>(circuit: &'a mut Circuit, path: &[usize]) -> &'a mut Circuit {
    path.iter().fold(circuit, |circuit, index| &mut circuit.children[*index])
}

/// A circuit placed in the diagram, by its depth, its column, and the middle of its box, see place.
struct Placed {
    depth: usize,
    y: f64,
    name: String,
    use_: Use,
    channels: Vec<String>,
    /// Its parent's place in the list, if it has one.
    parent: Option<usize>,
}

/// The height of a circuit's box, with a line for each of its channels, or for "the rest".
fn box_height(channels: usize) -> f64 {
    BOX_HEIGHT + channels.saturating_sub(1) as f64 * LINE_HEIGHT
}

/// Places circuit and those under it, each leaf below the last, from top, and each circuit level with the middle of
/// its children, or if its box is taller than they are, they level with it, returning the middle of its box. top
/// is moved below what was placed.
fn place(circuit: &Circuit, depth: usize, parent: Option<usize>, top: &mut f64, placed: &mut Vec<Placed>) -> f64 {
    let index = placed.len();
    placed.push(Placed { depth, y: 0.0, name: circuit.name.clone(), use_: circuit.use_, channels: circuit.channels.clone(), parent });
    let height = box_height(circuit.channels.len());
    let first_top = *top;
    let y = if circuit.children.is_empty() {
        *top += height + ROW_GAP;
        first_top + height / 2.0
    } else {
        let ys: Vec<f64> = circuit.children.iter().map(|child| place(child, depth + 1, Some(index), top, placed)).collect();
        let children_height = *top - ROW_GAP - first_top;
        if children_height < height {
            *top = first_top + height + ROW_GAP;
        }
        (ys[0] + ys[ys.len() - 1]) / 2.0
    };
    // At least half its height from the top of what it and its children take, so that it does not overlap the box
    // above it.
    let y = y.max(first_top + height / 2.0);
    placed[index].y = y;
    y
}

fn column_x(depth: usize) -> f64 {
    MARGIN + depth as f64 * (BOX_WIDTH + COLUMN_GAP)
}

/// The topology as a diagram, from the grid on the left to what it feeds on the right, each circuit in its use's colour
/// with the channels which measure it, or "the rest" if it has none, as it is its parent's less its siblings'.
#[component]
fn TopologyDiagram(topology: Circuit) -> Element {
    let mut placed = Vec::new();
    let mut top = MARGIN;
    place(&topology, 0, None, &mut top, &mut placed);
    let depth = placed.iter().map(|placed| placed.depth).max().unwrap_or(0);
    let width = column_x(depth) + BOX_WIDTH + MARGIN;
    let height = top - ROW_GAP + MARGIN;
    let placed_y: Vec<f64> = placed.iter().map(|placed| placed.y).collect();
    rsx! {
        svg {
            class: "topology-diagram",
            view_box: "0 0 {width} {height}",
            width: "{width}",
            role: "img",
            for (index, placed) in placed.iter().enumerate() {
                if let Some(parent) = placed.parent {
                    path {
                        key: "line{index}",
                        class: "topology-line",
                        d: {
                            let parent_y = placed_y[parent];
                            let (from, to) = (column_x(placed.depth - 1) + BOX_WIDTH, column_x(placed.depth));
                            let middle = (from + to) / 2.0;
                            format!("M {from} {parent_y} H {middle} V {} H {to}", placed.y)
                        },
                    }
                }
            }
            for (index, placed) in placed.iter().enumerate() {
                g { key: "box{index}",
                    rect {
                        class: "topology-box",
                        x: column_x(placed.depth),
                        y: placed.y - box_height(placed.channels.len()) / 2.0,
                        width: BOX_WIDTH,
                        height: box_height(placed.channels.len()),
                        rx: 8.0,
                        stroke: use_colour(placed.use_),
                    }
                    text {
                        class: "topology-name",
                        x: column_x(placed.depth) + 12.0,
                        y: placed.y - box_height(placed.channels.len()) / 2.0 + 21.0,
                        fill: use_colour(placed.use_),
                        "{placed.name}"
                    }
                    if placed.channels.is_empty() && placed.depth > 0 {
                        text {
                            class: "topology-channels rest",
                            x: column_x(placed.depth) + 12.0,
                            y: placed.y - box_height(0) / 2.0 + 41.0,
                            "the rest"
                        }
                    }
                    for (line, channel) in placed.channels.iter().enumerate() {
                        text {
                            key: "{channel}",
                            class: "topology-channels",
                            x: column_x(placed.depth) + 12.0,
                            y: placed.y - box_height(placed.channels.len()) / 2.0 + 41.0 + line as f64 * LINE_HEIGHT,
                            "{channel}"
                        }
                    }
                }
            }
        }
    }
}

/// Each circuit with its path and depth, the top first and each followed by those under it, for the list.
fn flattened(circuit: &Circuit, path: Vec<usize>, rows: &mut Vec<(Vec<usize>, Circuit)>) {
    rows.push((path.clone(), Circuit { children: Vec::new(), ..circuit.clone() }));
    for (index, child) in circuit.children.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        flattened(child, child_path, rows);
    }
}

/// The circuits as a list to edit: each one's name, use and channels, which are chosen from those the profile's
/// modules have, with buttons to add a circuit it feeds and to remove it.
#[component]
fn TopologyEditor(edited: Signal<Option<Circuit>>, channels: Vec<Channel>) -> Element {
    let Some(topology) = edited() else {
        return rsx! {};
    };
    let mut rows = Vec::new();
    flattened(&topology, Vec::new(), &mut rows);
    rsx! {
        table { class: "topology-editor",
            thead {
                tr {
                    th { "Circuit" }
                    th { "For" }
                    th { "Measured by" }
                    th {}
                }
            }
            tbody {
                for (path, circuit) in rows {
                    tr { key: "{path:?}",
                        td {
                            style: "padding-left: {path.len() as f64 * 1.5 + 0.3}em",
                            input {
                                r#type: "text",
                                value: "{circuit.name}",
                                oninput: {
                                    let path = path.clone();
                                    move |event: FormEvent| {
                                        if let Some(topology) = edited.write().as_mut() {
                                            at(topology, &path).name = event.value();
                                        }
                                    }
                                },
                            }
                        }
                        td {
                            select {
                                value: use_id(circuit.use_),
                                onchange: {
                                    let path = path.clone();
                                    move |event: FormEvent| {
                                        let chosen = Use::ALL.into_iter().find(|use_| use_id(*use_) == event.value());
                                        if let (Some(topology), Some(chosen)) = (edited.write().as_mut(), chosen) {
                                            at(topology, &path).use_ = chosen;
                                        }
                                    }
                                },
                                for use_ in Use::ALL {
                                    option { value: use_id(use_), selected: use_ == circuit.use_, "{use_.label()}" }
                                }
                            }
                        }
                        td { class: "topology-channel-list",
                            for channel in channel_choices(&channels, &circuit) {
                                button {
                                    r#type: "button",
                                    class: if circuit.channels.contains(&channel.id) { "channel-chip on" } else { "channel-chip" },
                                    title: "{channel.description}",
                                    onclick: {
                                        let (path, id) = (path.clone(), channel.id.clone());
                                        move |_| {
                                            if let Some(topology) = edited.write().as_mut() {
                                                let channels = &mut at(topology, &path).channels;
                                                match channels.iter().position(|channel| *channel == id) {
                                                    Some(index) => { channels.remove(index); },
                                                    None => channels.push(id.clone()),
                                                }
                                            }
                                        }
                                    },
                                    "{channel.id}"
                                }
                            }
                        }
                        td { class: "topology-actions",
                            button {
                                r#type: "button",
                                onclick: {
                                    let path = path.clone();
                                    move |_| {
                                        if let Some(topology) = edited.write().as_mut() {
                                            at(topology, &path).children.push(Circuit { name: String::from("New circuit"), ..Circuit::default() });
                                        }
                                    }
                                },
                                "Add fed circuit"
                            }
                            if let Some((last, parent)) = path.split_last() {
                                button {
                                    r#type: "button",
                                    onclick: {
                                        let (last, parent) = (*last, parent.to_vec());
                                        move |_| {
                                            if let Some(topology) = edited.write().as_mut() {
                                                at(topology, &parent).children.remove(last);
                                            }
                                        }
                                    },
                                    "Remove"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The channels a circuit can be measured by: the profile's modules', and any it has which none of them has now,
/// e.g. of a module since taken out of the profile, so that they can be taken off it.
fn channel_choices(channels: &[Channel], circuit: &Circuit) -> Vec<Channel> {
    let mut choices = channels.to_vec();
    for id in &circuit.channels {
        if !choices.iter().any(|channel| channel.id == *id) {
            choices.push(Channel { id: id.clone(), description: String::from("No module of this profile has this channel now"), use_: circuit.use_ });
        }
    }
    choices
}

/// The site's topology, drawn, and to edit, see the module comment.
#[component]
pub fn SitePage() -> Element {
    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let registrations = use_context::<ModuleRegistrations>();
    let mut settings = use_context::<Signal<ProfileSettings>>();
    let mut edited = use_signal(|| settings.peek().topology.clone());
    let mut status = use_signal(|| None::<Result<String, String>>);
    let found = use_resource(move || {
        let context = context_signal.read().clone();
        let registrations = registrations.clone();
        async move {
            let found = super::electricity::find(context.ok_or_else(|| anyhow!("There is no profile"))?, registrations).await?;
            anyhow::Ok((found.channels.clone(), found.not_ready.clone()))
        }
    });
    let (channels, not_ready) = match &*found.read() {
        None => return rsx! { h1 { "Site" } p { "Loading..." } },
        Some(Err(error)) => return rsx! {
            h1 { "Site" }
            div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the modules" } }
        },
        Some(Ok(found)) => found.clone(),
    };
    let profile_name = context_signal.read().as_ref().map(|context| context.profile.active_profile.name.clone()).unwrap_or_default();
    let saved = settings.read().topology.clone();
    let changed = edited() != saved;
    let problems = edited().map(|topology| topology.problems()).unwrap_or_default();
    // Saves the topology, or clears it if it is None.
    let save = move |topology: Option<Circuit>| {
        let profile_name = profile_name.clone();
        move |_: MouseEvent| {
            if topology.is_none() {
                edited.set(None);
            }
            match crate::profile::update_topology(&profile_name, topology.as_ref()) {
                Ok(()) => {
                    settings.write().topology = topology.clone();
                    status.set(Some(Ok(String::from(if topology.is_some() { "Saved" } else { "Cleared" }))));
                },
                Err(error) => status.set(Some(Err(format!("Unable to save the topology, {error:#}")))),
            }
        }
    };
    let suggested = Circuit::suggested(&channels);

    rsx! {
        h1 { "Site" }
        p {
            "Where in the house each module measures, from the grid connection, on the left, through the circuits each one "
            "feeds. A circuit with no channel has the rest of what feeds it, e.g. the car's charger beside a Tesla gateway has the grid's import less the "
            "gateway's. The Electricity page splits the grid's import into the car's, the batteries' and the house's "
            "with it."
        }
        for module_id in not_ready.iter() {
            p { "Log in to {module_id} to see its channels here." }
        }
        match edited() {
            Some(topology) => rsx! {
                div { class: "topology-scroll", TopologyDiagram { topology } }
                for problem in problems.iter() {
                    p { class: "invalid", "{problem}" }
                }
                TopologyEditor { edited, channels: channels.clone() }
            },
            None => rsx! {
                p { "The site's topology is not set, so the grid's import is shown by rate only. A first one from this profile's channels:" }
                div { class: "topology-scroll", TopologyDiagram { topology: suggested.clone() } }
            },
        }
        div { class: "topology-buttons",
            if edited().is_none() {
                button { r#type: "button", onclick: move |_| edited.set(Some(suggested.clone())), "Start from this" }
            } else {
                button {
                    r#type: "button",
                    disabled: !changed || !problems.is_empty(),
                    onclick: save(edited()),
                    "Save"
                }
                button { r#type: "button", disabled: !changed, onclick: move |_| edited.set(saved.clone()), "Undo changes" }
                if saved.is_some() {
                    button { r#type: "button", onclick: save(None), "Clear" }
                }
            }
        }
        match status() {
            Some(Ok(message)) => rsx! { p { class: "ok", "{message}" } },
            Some(Err(message)) => rsx! { p { class: "invalid", "{message}" } },
            None => rsx! {},
        }
        if !channels.is_empty() {
            h2 { "Channels" }
            table {
                for channel in channels.iter() {
                    tr {
                        td { "{channel.id}" }
                        td { "{channel.description}" }
                    }
                }
            }
        }
    }
}
