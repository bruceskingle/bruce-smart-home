//! Home's Site page: the site's topology, where in the house each module's channels measure, see power::topology, drawn
//! as a diagram from the grid to the house, and edited as a list of its circuits, which is saved with the profile's
//! settings. The Electricity page splits the grid's import by use with it. The topology being edited is checked against
//! the last CHECK_DAYS days' measurements, which also say how the channels relate, whatever the topology, see
//! power::check.

use std::sync::Arc;

use anyhow::anyhow;
use dioxus::prelude::*;

use std::collections::BTreeMap;

use crate::components::category_filter::{category_colour, use_colour_store};
use crate::components::date_range_picker::{LocalSlot, RangeSelection};
use crate::components::error_text::ErrorText;
use crate::data_set::CancelToken;
use crate::period::Period;
use crate::power::check::{self, Check, CheckKind, Relation, Series};
use crate::power::topology::{Channel, Circuit, Use};
use crate::util::SharedProp;
use crate::views::electricity::Found;
use crate::profile::Settings as ProfileSettings;
use crate::{MarcoSparkoContext, ModuleRegistrations};

/// The size of a circuit's box in the diagram, with one channel, and the gaps between them, in the SVG's units. Each
/// channel is a line of its own, see box_height.
pub(super) const BOX_WIDTH: f64 = 190.0;
pub(super) const BOX_HEIGHT: f64 = 54.0;
pub(super) const LINE_HEIGHT: f64 = 16.0;
pub(super) const COLUMN_GAP: f64 = 70.0;
pub(super) const ROW_GAP: f64 = 18.0;
pub(super) const MARGIN: f64 = 8.0;

/// The colour of a circuit: a kind's, or its category's, from colours, the profile's categories', see
/// category_colours, as the Electricity page's labels have them, e.g. the car's is EV Low Rate's.
pub(super) fn use_colour(use_: &Use, colours: &BTreeMap<String, String>) -> String {
    match use_ {
        Use::Grid => String::from("#b0b0b0"),
        Use::Board => String::from("#e0e0e0"),
        Use::Solar => String::from("#f2c94c"),
        other => other.category().and_then(|category| colours.get(category)).cloned().unwrap_or_else(|| String::from("#8a8a99")),
    }
}

/// The colour of each of the profile's categories and the batteries', see category_filter::category_colour.
pub(super) fn category_colours(settings: &ProfileSettings) -> BTreeMap<String, String> {
    let categories = settings.all_categories();
    categories.iter().map(String::as_str).chain([crate::power::topology::BATTERY])
        .map(|category| (category.to_string(), category_colour(category, &settings.colours, &categories)))
        .collect()
}

/// A use as the value of an option, which a category's name can not be mistaken for, e.g. load:Car.
fn use_id(use_: &Use) -> String {
    match use_ {
        Use::Load(category) => format!("load:{category}"),
        kind => String::from(kind.clone()),
    }
}

/// The circuit at path, the indexes of the children down to it, from circuit.
fn at<'a>(circuit: &'a mut Circuit, path: &[usize]) -> &'a mut Circuit {
    path.iter().fold(circuit, |circuit, index| &mut circuit.children[*index])
}

/// A circuit placed in the diagram, by its depth, its column, and the middle of its box, see place.
pub(super) struct Placed {
    pub(super) depth: usize,
    pub(super) y: f64,
    pub(super) name: String,
    pub(super) use_: Use,
    pub(super) channels: Vec<String>,
    /// Its parent's place in the list, if it has one.
    pub(super) parent: Option<usize>,
}

/// The height of a circuit's box, with a line for each of its channels, or for "the rest".
pub(super) fn box_height(channels: usize) -> f64 {
    BOX_HEIGHT + channels.saturating_sub(1) as f64 * LINE_HEIGHT
}

/// Places circuit and those under it, each leaf below the last, from top, and each circuit level with the middle of
/// its children, or if its box is taller than they are, they level with it, returning the middle of its box. top
/// is moved below what was placed.
pub(super) fn place(circuit: &Circuit, depth: usize, parent: Option<usize>, top: &mut f64, placed: &mut Vec<Placed>) -> f64 {
    let index = placed.len();
    placed.push(Placed { depth, y: 0.0, name: circuit.name.clone(), use_: circuit.use_.clone(), channels: circuit.channels.clone(), parent });
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

pub(super) fn column_x(depth: usize) -> f64 {
    MARGIN + depth as f64 * (BOX_WIDTH + COLUMN_GAP)
}

/// The topology as a diagram, from the grid on the left to what it feeds on the right, each circuit in its use's colour
/// with the channels which measure it, or "the rest" if it has none, as it is its parent's less its siblings'.
#[component]
fn TopologyDiagram(topology: Circuit, colours: BTreeMap<String, String>) -> Element {
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
                        stroke: use_colour(&placed.use_, &colours),
                    }
                    text {
                        class: "topology-name",
                        x: column_x(placed.depth) + 12.0,
                        y: placed.y - box_height(placed.channels.len()) / 2.0 + 21.0,
                        fill: use_colour(&placed.use_, &colours),
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
fn TopologyEditor(edited: Signal<Option<Circuit>>, channels: Vec<Channel>, categories: Vec<String>) -> Element {
    let uses = Use::all(&categories);
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
                                value: use_id(&circuit.use_),
                                onchange: {
                                    let (path, uses) = (path.clone(), uses.clone());
                                    move |event: FormEvent| {
                                        let chosen = uses.iter().find(|use_| use_id(use_) == event.value()).cloned();
                                        if let (Some(topology), Some(chosen)) = (edited.write().as_mut(), chosen) {
                                            at(topology, &path).use_ = chosen;
                                        }
                                    }
                                },
                                for use_ in uses.iter() {
                                    option { value: use_id(use_), selected: *use_ == circuit.use_, "{use_.label()}" }
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
            choices.push(Channel { id: id.clone(), description: String::from("No module of this profile has this channel now"), use_: circuit.use_.clone() });
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
            anyhow::Ok(Arc::new(found))
        }
    });
    let checked = use_signal(|| None::<Result<SharedProp<Measured>, String>>);
    let checking = use_signal(|| false);
    let progress = use_signal(String::new);
    // With no topology, the measurements are checked straight away, so that the first one is what they say, see
    // suggested below.
    use_effect(move || {
        if let Some(Ok(found)) = &*found.read()
            && edited.peek().is_none() && checked.peek().is_none() && !*checking.peek()
        {
            start_check(found.clone(), checked, checking, progress);
        }
    });
    let (channels, not_ready, found) = match &*found.read() {
        None => return rsx! { h1 { "Site" } p { "Loading..." } },
        Some(Err(error)) => return rsx! {
            h1 { "Site" }
            div { class: "error", ErrorText { error: format!("{error:#}"), message: "Failed to load the modules" } }
        },
        Some(Ok(found)) => (found.channels.clone(), found.not_ready.clone(), found.clone()),
    };
    let check = move |_: MouseEvent| start_check(found.clone(), checked, checking, progress);
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
    // The first topology, if none is set: the one from what the channels usually measure, if the measurements fit it,
    // otherwise the one they say, if they fit that, see check::propose, otherwise the first, until they are checked.
    let guessed = Circuit::suggested(&channels);
    let fits = |topology: &Circuit, measured: &Measured| check::verdict(&check::checks(topology, &measured.series)) == check::Verdict::LooksRight;
    let (suggested, measured_suggestion) = match checked() {
        Some(Ok(measured)) if fits(&guessed, &measured.0) => (guessed, true),
        Some(Ok(measured)) => match check::propose(&guessed, &measured.0.series, &measured.0.relations, &channels)
            .filter(|proposed| fits(proposed, &measured.0))
        {
            Some(proposed) => (proposed, true),
            None => (guessed, false),
        },
        _ => (guessed, false),
    };
    let categories = settings.read().all_categories();
    let colours = category_colours(&settings.read());

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
                div { class: "topology-scroll", TopologyDiagram { topology, colours: colours.clone() } }
                for problem in problems.iter() {
                    p { class: "invalid", "{problem}" }
                }
                TopologyEditor { edited, channels: channels.clone(), categories: categories.clone() }
            },
            None => rsx! {
                p {
                    "The site's topology is not set, so the grid's import is shown by rate only. "
                    if measured_suggestion {
                        "A first one, which the measurements of the last {CHECK_DAYS} days fit:"
                    } else if checking() {
                        "A first one from what this profile's channels usually measure, while the measurements are checked:"
                    } else {
                        "A first one from what this profile's channels usually measure, which the measurements do not say is right:"
                    }
                }
                div { class: "topology-scroll", TopologyDiagram { topology: suggested.clone(), colours: colours.clone() } }
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
        CategoryEditor { edited }
        h2 { "Checks" }
        p {
            "What the channels measured over the last {CHECK_DAYS} days, half hour by half hour, against the topology "
            "above, as it is edited, and what they say of each other, whatever the topology."
        }
        div { class: "topology-buttons",
            button { r#type: "button", disabled: checking(), onclick: check,
                if checked().is_some() { "Check again" } else { "Check" }
            }
        }
        if checking() {
            p { "Loading... {progress}" }
        }
        match checked() {
            Some(Ok(measured)) => rsx! {
                if let Some(topology) = edited() {
                    TopologyChecks { topology, measured: measured.clone(), edited, channels: channels.clone(), colours: colours.clone() }
                }
                ChannelRelations { measured }
            },
            Some(Err(error)) => rsx! { div { class: "error", ErrorText { error, message: "Failed to load the measurements" } } },
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

/// How many days the checks are of, up to today.
const CHECK_DAYS: i64 = 7;

/// The channels' half hours of the last CHECK_DAYS days, and what they say of each other, see power::check, with the
/// timezone to show times in, the first source's, e.g. the supplier's.
pub struct Measured {
    series: Series,
    relations: Vec<Relation>,
    tz: &'static time_tz::Tz,
}

/// The channels' net import each half hour of the last CHECK_DAYS days: the grid readings', from the sources, put
/// together as on the Electricity page, as the supplier's channels', and each of the measurements'.
async fn measure(found: Arc<Found>, mut progress: Signal<String>) -> anyhow::Result<Measured> {
    let tz = found.sources.first().map(|source| source.timezone()).unwrap_or(time_tz::timezones::db::UTC);
    let range = RangeSelection { start: LocalSlot { date: crate::clock::today(tz) - time::Duration::days(CHECK_DAYS - 1), slot: 0 }, end: None };
    let (start, end) = range.to_date_times(tz).map_err(|error| anyhow!(error))?;
    let mut loaded = Vec::new();
    for source in &found.sources {
        loaded.push(source.load(start, end, None, progress, CancelToken::new()).await?);
    }
    let (start, end) = (start.unix_timestamp(), end.unix_timestamp());
    let loaded_refs: Vec<&dyn crate::power::LoadedElectricity> = loaded.iter().map(|loaded| loaded.as_ref()).collect();
    let electricity = crate::power::electricity(&loaded_refs, None, start, end, Period::HalfHour, tz);
    let grid = electricity.points.iter()
        .filter(|point| point.end - point.start == crate::power::measured::HALF_HOUR)
        .map(|point| (point.start, point.import.iter().sum::<f64>() - point.export))
        .collect();
    let mut measurements = Vec::new();
    for source in &found.measurements {
        progress.set(String::from("Loading the measurements"));
        measurements.push(source.load(start, end).await?);
    }
    // The grid readings are of the channels which measure the grid which no device's measurements have, e.g. the
    // supplier's meter.
    let measured: std::collections::BTreeSet<String> = measurements.iter().flat_map(|measurements| measurements.channels()).collect();
    let grid_name = found.channels.iter()
        .filter(|channel| channel.use_ == Use::Grid && !measured.contains(&channel.id))
        .map(|channel| channel.id.clone())
        .next()
        .unwrap_or_else(|| String::from("the grid readings"));
    let series = Series::new(start, end, &grid, &grid_name, &measurements);
    let relations = check::relations(&series);
    Ok(Measured { series, relations, tz })
}

/// A half hour's start, as the checks show it, e.g. Tue 3 Nov 04:00.
fn half_hour_label(start: i64, tz: &'static time_tz::Tz) -> String {
    use time_tz::OffsetDateTimeExt;
    let format = time::macros::format_description!("[weekday repr:short] [day padding:none] [month repr:short] [hour]:[minute]");
    time::OffsetDateTime::from_unix_timestamp(start).ok()
        .and_then(|time| time.to_timezone(tz).format(&format).ok())
        .unwrap_or_default()
}

/// How far out a check is, as a percentage, in red if it is far enough out to be worth looking at, see check::SAME.
fn mismatch_cell(mismatch: Option<f64>) -> Element {
    match mismatch {
        Some(mismatch) => {
            let class = if mismatch <= 0.05 { "ok" } else if mismatch > 0.15 { "invalid" } else { "" };
            rsx! { td { class, {format!("{:.1}%", mismatch * 100.0)} } }
        },
        None => rsx! { td {} },
    }
}

/// The checks of topology, see power::check::checks: whether it looks right, and if not, one from the measurements
/// which does, if they have one, to use instead, which sets edited, then for each check, the half hours it has, the two
/// sides' totals, how far out they are, and the half hours furthest out.
#[component]
fn TopologyChecks(topology: Circuit, measured: SharedProp<Measured>, edited: Signal<Option<Circuit>>, channels: Vec<Channel>,
    colours: BTreeMap<String, String>) -> Element
{
    let checks = check::checks(&topology, &measured.0.series);
    let verdict = check::verdict(&checks);
    // One which fits, if this does not, and the measurements say of one.
    let proposed = match &verdict {
        check::Verdict::Wrong(_) => check::propose(&topology, &measured.0.series, &measured.0.relations, &channels)
            .filter(|proposed| *proposed != topology && check::verdict(&check::checks(proposed, &measured.0.series)) == check::Verdict::LooksRight),
        _ => None,
    };
    let verdict_text = match &verdict {
        check::Verdict::NothingToCheck => rsx! { p { "There is nothing to check yet." } },
        check::Verdict::LooksRight => rsx! {
            p { class: "ok", "The topology looks right: the measurements fit it, each check to within 10%." }
        },
        check::Verdict::Wrong(wrong) => rsx! {
            p { class: "invalid",
                {format!("The topology does not fit the measurements: {} of the checks below {} out.", wrong.len(), if wrong.len() == 1 { "is" } else { "are" })}
            }
            match proposed.clone() {
                Some(proposed) => rsx! {
                    p { "This topology, from what the measurements say, fits them:" }
                    div { class: "topology-scroll", TopologyDiagram { topology: proposed.clone(), colours: colours.clone() } }
                    div { class: "topology-buttons",
                        button { r#type: "button", onclick: move |_| edited.set(Some(proposed.clone())), "Use this" }
                    }
                    p { "Use this puts it in the editor, to change or Save." }
                },
                None => rsx! { p { "The measurements do not say of a topology which fits better. Check where each channel is." } },
            }
        },
    };
    let tz = measured.0.tz;
    let worst = |check: &Check| -> String {
        check.worst.iter()
            .map(|(start, left, right)| match check.kind {
                CheckKind::Rest => format!("{} {:.2}", half_hour_label(*start, tz), left - right),
                _ => format!("{} {:.2} against {:.2}", half_hour_label(*start, tz), left, right),
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    if checks.is_empty() {
        return rsx! { p { "There is nothing to check: no circuit has its own channel and those of all the circuits it feeds, or two channels." } };
    }
    rsx! {
        {verdict_text}
        table { class: "topology-checks",
            thead {
                tr {
                    th { "Check" }
                    th { "Half hours" }
                    th { "kWh" }
                    th { "Out by" }
                    th { "Furthest out, kWh" }
                }
            }
            tbody {
                for (index, check) in checks.iter().enumerate() {
                    tr { key: "{index}",
                        td { "{check.title}" }
                        td { "{check.half_hours}" }
                        td {
                            match check.kind {
                                CheckKind::Rest => format!("{} {:.1} less {} {:.1}, the rest {:.1}", check.left, check.left_total, check.right,
                                    check.right_total, check.left_total - check.right_total),
                                _ => format!("{} {:.1}, {} {:.1}", check.left, check.left_total, check.right, check.right_total),
                            }
                        }
                        if check.kind == CheckKind::Rest {
                            td { class: if check.negative > 0 { "invalid" } else { "ok" },
                                if check.negative > 0 { "{check.negative} half hours less than none" } else { "never less than none" }
                            }
                        } else {
                            {mismatch_cell(check.mismatch)}
                        }
                        td { {worst(check)} }
                    }
                }
            }
        }
    }
}

/// What the measurements say of how the channels relate, see power::check::relations, to help place them.
#[component]
fn ChannelRelations(measured: SharedProp<Measured>) -> Element {
    let relations = &measured.0.relations;
    let percent = |mismatch: f64| format!("{:.1}%", mismatch * 100.0);
    // A channel in a sum, with those which are the same, as a sum names only one of them.
    let named = |name: &String| -> String {
        let same: Vec<&str> = relations.iter()
            .filter_map(|relation| match relation {
                Relation::Same { a, b, .. } if a == name => Some(b.as_str()),
                Relation::Same { a, b, .. } if b == name => Some(a.as_str()),
                _ => None,
            })
            .collect();
        if same.is_empty() { name.clone() } else { format!("{name} (the same as {})", same.join(", ")) }
    };
    rsx! {
        h3 { "What the measurements say" }
        if relations.is_empty() {
            p { "No two channels measure the same thing, and none is the sum of others." }
        } else {
            ul {
                for (index, relation) in relations.iter().enumerate() {
                    li { key: "{index}",
                        match relation {
                            Relation::Same { a, b, mismatch, half_hours } =>
                                format!("{a} and {b} measure the same circuit: they are {} apart over {half_hours} half hours.", percent(*mismatch)),
                            Relation::Reversed { a, b, mismatch, half_hours } =>
                                format!("{b} measures what {a} does, the other way round, to within {} over {half_hours} half hours: is its clamp on backwards?", percent(*mismatch)),
                            Relation::Sum { a, parts, mismatch, half_hours } =>
                                format!("{} is {}, to within {} over {half_hours} half hours, as if it feeds them.", named(a),
                                    parts.iter().map(named).collect::<Vec<_>>().join(" + "), percent(*mismatch)),
                        }
                    }
                }
            }
        }
    }
}

/// The categories of the profile's loads, see power::topology::Use::Load, each with its colour, which is its labels'
/// default in the graphs, to choose, and a way to add more, which saves them with the profile, see
/// profile::update_settings_with. One which is not built in can be removed if no circuit of edited is of it.
#[component]
fn CategoryEditor(edited: Signal<Option<Circuit>>) -> Element {
    let mut settings = use_context::<Signal<ProfileSettings>>();
    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let store = use_colour_store();
    let mut new_category = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let profile_name = context_signal.read().as_ref().map(|context| context.profile.active_profile.name.clone()).unwrap_or_default();
    let all = settings.read().all_categories();
    let colours = category_colours(&settings.read());
    let used = edited().map(|topology| topology.categories()).unwrap_or_default();
    // Saves a change to the categories, and shows them as saved.
    let mut change_categories = move |profile_name: String, change: Box<dyn FnOnce(&mut Vec<String>)>| {
        match crate::profile::update_settings_with(&profile_name, |saved| change(&mut saved.categories)) {
            Ok(saved) => {
                settings.write().categories = saved.categories;
                error.set(None);
            },
            Err(failed) => error.set(Some(format!("Unable to save the categories, {failed:#}"))),
        }
    };
    let add = {
        let (profile_name, all) = (profile_name.clone(), all.clone());
        move || {
            let name = new_category.read().trim().to_string();
            if !crate::power::topology::is_valid_category(&name) {
                error.set(Some(String::from("A category needs a name, which is not grid, board, battery or solar")));
            } else if all.iter().any(|category| category.eq_ignore_ascii_case(&name)) {
                error.set(Some(format!("There is a category {name} already")));
            } else {
                change_categories(profile_name.clone(), Box::new(move |categories| categories.push(name)));
                new_category.set(String::new());
            }
        }
    };
    rsx! {
        h2 { "Categories" }
        p {
            "What the loads are for, which is what the Electricity page splits the import into, e.g. EV Low Rate for the "
            "car's. The colour is the default of the category's labels in the graphs, which their Categories menus can change."
        }
        table { class: "category-editor",
            for category in all.iter() {
                tr { key: "{category}",
                    td {
                        input {
                            r#type: "color",
                            value: "{colours.get(category).cloned().unwrap_or_default()}",
                            onclick: move |_| crate::components::color_panel::move_color_panel_to_mouse(),
                            oninput: {
                                let category = category.clone();
                                move |event: FormEvent| store.save(&category, &event.value())
                            },
                        }
                    }
                    td { "{category}" }
                    td {
                        if crate::power::topology::BUILT_IN_CATEGORIES.contains(&category.as_str()) {
                            span { class: "muted", "built in" }
                        } else if used.contains(category) {
                            span { class: "muted", "in use" }
                        } else {
                            button {
                                r#type: "button",
                                onclick: {
                                    let (profile_name, category) = (profile_name.clone(), category.clone());
                                    move |_| change_categories(profile_name.clone(), Box::new({
                                        let category = category.clone();
                                        move |categories| categories.retain(|known| *known != category)
                                    }))
                                },
                                "Remove"
                            }
                        }
                    }
                }
            }
        }
        div { class: "topology-buttons",
            input {
                r#type: "text",
                placeholder: "New category, e.g. Hot water",
                value: "{new_category}",
                oninput: move |event| new_category.set(event.value()),
                onkeydown: {
                    let mut add = add.clone();
                    move |event: KeyboardEvent| if event.key().to_string() == "Enter" { add() }
                },
            }
            button { r#type: "button", onclick: { let mut add = add.clone(); move |_| add() }, "Add category" }
        }
        if let Some(error) = error() {
            p { class: "invalid", "{error}" }
        }
    }
}

/// Loads and checks the last CHECK_DAYS days' measurements, see measure, into checked, with checking set meanwhile.
fn start_check(found: Arc<Found>, mut checked: Signal<Option<Result<SharedProp<Measured>, String>>>, mut checking: Signal<bool>,
    progress: Signal<String>)
{
    checking.set(true);
    spawn(async move {
        let result = measure(found, progress).await.map(|measured| SharedProp(Arc::new(measured))).map_err(|error| format!("{error:#}"));
        checked.set(Some(result));
        checking.set(false);
    });
}
