//! The Capture Devices page: the ESP32s the profile has adopted, see capture_device.rs, with their state, settings,
//! restart and release, and adopting another.

use std::collections::BTreeMap;
use std::sync::Arc;

use dioxus::prelude::*;
use serde_json::{Map, Value, json};

use crate::MarcoSparkoContext;
use crate::capture_device::{self, DeviceClient, DeviceProfile, FoundDevice};
use crate::components::error_text::ErrorText;
use crate::profile::Settings as ProfileSettings;

/// A setting changed on the page but not yet saved, by feature and setting name.
type Edits = BTreeMap<(String, String), Value>;

/// The settings of a device's features which the profile's modules know, as (feature, module, [(setting, the module
/// profile's field)]), see FeatureSettings' Fill In.
const FROM_PROFILE: &[(&str, &str, &[(&str, &str)])] = &[
    ("TeslaCapture", "tesla", &[("tesla_host", "gatewayHost"), ("tesla_email", "email"), ("tesla_password", "password")]),
    ("MyenergiCapture", "myenergi", &[("myenergi_hub", "hubSerial"), ("myenergi_key", "apiKey")]),
    ("OctopusCapture", "octopus", &[("octopus_key", "apiKey")]),
];

/// What the profile's module knows of a device feature's settings, see FROM_PROFILE, as edits.
fn from_profile(profile_name: &str, feature: &str) -> Edits {
    let Some((_, module, fields)) = FROM_PROFILE.iter().find(|(name, _, _)| *name == feature) else {
        return Edits::new();
    };
    let profile = crate::profile::fetch_active_profile(&Some(profile_name.to_string())).ok();
    let module_profile = profile.as_ref().and_then(|profile| profile.active_profile.modules.get(*module));
    let mut edits: Edits = fields.iter()
        .filter_map(|(setting, field)| {
            let value = module_profile?.get(*field)?.as_str()?;
            Some(((feature.to_string(), setting.to_string()), json!(value)))
        })
        .collect();
    // The account the Octopus module uses, its default, the first of its cached viewer's, see ViewerDataSet.
    if feature == "OctopusCapture" {
        let viewer = crate::MarcoSparkoContext::get_cache_dir_path().ok()
            .map(|dir| dir.join(format!("{profile_name}-octopus")).join("#Viewer"))
            .and_then(|path| crate::cache::CacheConfig::read_json(&path));
        if let Some(account) = viewer.as_ref().and_then(|viewer| viewer["viewer"]["accounts"][0]["number"].as_str()) {
            edits.insert((feature.to_string(), "octopus_account".to_string()), json!(account));
        }
    }
    edits
}

/// The modules which read capture devices, see capture_device::CaptureReader, which are polled as soon as a device is
/// adopted, so that the app has what it took straight away.
const READING_MODULES: &[&str] = &["tesla", "myenergi", "octopus"];

/// Saves the profile's devices, in the profile and for the pages which show them.
fn save_devices(profile_name: &str, settings: &mut Signal<ProfileSettings>, devices: Vec<DeviceProfile>) -> Result<(), String> {
    crate::profile::update_capture_devices(profile_name, &devices).map_err(|error| format!("Unable to save the devices, {error:#}"))?;
    settings.write().capture_devices = devices;
    Ok(())
}

fn status_line(status: Option<Result<String, String>>) -> Element {
    match status {
        Some(Ok(message)) => rsx! { p { class: "ok", "{message}" } },
        Some(Err(message)) => rsx! { p { class: "invalid", "{message}" } },
        None => rsx! {},
    }
}

/// How often each device card asks its device what it is, see DeviceCard.
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(30);

/// How many days of a device's starts are summarised, see restarts.
const RESTART_DAYS: i64 = 7;

/// A device's starts in the last RESTART_DAYS, see restarts.
#[derive(Debug, PartialEq)]
struct Restarts {
    count: usize,
    /// How many of each kind, e.g. 1 crash, in the order of KINDS, and whether that kind is trouble.
    kinds: Vec<(String, bool)>,
    /// How many more there were, which the device could not log, which show as jumps in their numbers, see
    /// capture_device::Start::count.
    not_logged: u32,
    /// Each, newest first: when, in the computer's time zone, its number, what it was after, see started_after,
    /// whether that is trouble, and how many before it were not logged.
    list: Vec<(String, Option<u32>, String, bool, u32)>,
}

/// The kinds of start, by reset reason, as a noun and its plural, for the summary, see restarts.
const KINDS: [(&[&str], &str, &str); 8] = [
    (&["Panic", "CpuLockup"], "crash", "crashes"),
    (&["InterruptWatchdog", "TaskWatchdog", "OtherWatchdog"], "watchdog reset", "watchdog resets"),
    (&["Brownout", "PowerGlitch"], "power dip", "power dips"),
    (&["Software"], "restart", "restarts"),
    (&["PowerOn"], "switch-on", "switch-ons"),
    (&["ExternalPin"], "reset button press", "reset button presses"),
    (&["DeepSleep"], "wake from sleep", "wakes from sleep"),
    (&["Usb", "Jtag"], "reset over USB", "resets over USB"),
];

/// A device's starts in the last RESTART_DAYS before now.
fn restarts(starts: &capture_device::Starts, now: chrono::DateTime<chrono::Utc>) -> Restarts {
    let from = now - chrono::TimeDelta::days(RESTART_DAYS);
    let recent: Vec<(chrono::DateTime<chrono::Utc>, &capture_device::Start)> = starts.starts.iter()
        .filter_map(|start| Some((chrono::DateTime::parse_from_rfc3339(&start.time).ok()?.to_utc(), start)))
        .filter(|(time, _)| *time >= from)
        .collect();
    // Not logged before each, from the jump in their numbers.
    let gaps: Vec<u32> = std::iter::once(0)
        .chain(recent.windows(2).map(|pair| match (pair[0].1.count, pair[1].1.count) {
            (Some(before), Some(after)) => after.saturating_sub(before).saturating_sub(1),
            _ => 0,
        }))
        .collect();
    let mut kinds = Vec::new();
    for (reasons, one, many) in KINDS {
        let n = recent.iter().filter(|(_, start)| reasons.contains(&start.reason.as_str())).count();
        if n > 0 {
            kinds.push((format!("{n} {}", if n == 1 { one } else { many }), started_after(reasons[0]).1));
        }
    }
    let other = recent.iter().filter(|(_, start)| !KINDS.iter().any(|(reasons, _, _)| reasons.contains(&start.reason.as_str()))).count();
    if other > 0 {
        kinds.push((format!("{other} other"), true));
    }
    let list = recent.iter().zip(&gaps).rev()
        .map(|((time, start), gap)| {
            let (text, trouble) = started_after(&start.reason);
            (time.with_timezone(&chrono::Local).format("%a %-d %b %H:%M:%S").to_string(), start.count, text, trouble, *gap)
        })
        .collect();
    Restarts { count: recent.len(), kinds, not_logged: gaps.iter().sum(), list }
}

/// What a device's reset reason, see DeviceInfo::reset_reason, means, and whether it is trouble, e.g. a crash, rather
/// than being switched on or restarted.
pub(crate) fn started_after(reason: &str) -> (String, bool) {
    match reason {
        "PowerOn" => ("After being switched on".to_string(), false),
        "Software" => ("After being restarted".to_string(), false),
        "ExternalPin" => ("After its reset button was pressed".to_string(), false),
        "DeepSleep" => ("After waking from deep sleep".to_string(), false),
        "Usb" | "Jtag" => ("After a reset over USB, e.g. being flashed".to_string(), false),
        "Panic" => ("After a crash (panic)".to_string(), true),
        "InterruptWatchdog" | "TaskWatchdog" | "OtherWatchdog" =>
            (format!("After it stopped responding ({reason})"), true),
        "Brownout" | "PowerGlitch" => ("After a dip in its power supply (brownout)".to_string(), true),
        "CpuLockup" => ("After a crash (CPU lockup)".to_string(), true),
        _ => (format!("After {reason}"), true),
    }
}

fn up_time(seconds: u64) -> String {
    match seconds {
        0..120 => format!("{seconds} s"),
        120..7200 => format!("{} min", seconds / 60),
        7200..172800 => format!("{} h", seconds / 3600),
        _ => format!("{} days", seconds / 86400),
    }
}

#[component]
pub fn Devices() -> Element {
    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let settings = use_context::<Signal<ProfileSettings>>();
    let profile_name = context_signal.read().as_ref().map(|context| context.profile.active_profile.name.clone()).unwrap_or_default();
    let devices = settings.read().capture_devices.clone();
    // The device whose card is shown, below the table, see DeviceRow.
    let selected = use_signal(|| None::<Selected>);

    rsx! {
        div { class: "settings-page",
            h1 { "Capture Devices" }
            p { class: "note",
                "Capture devices are small always-on ESP32 boards which fetch data while this computer sleeps. Once adopted, only this profile can change their settings."
            }
            if devices.is_empty() {
                p { "There are no capture devices in profile {profile_name} yet." }
            } else {
                table { class: "display device-list",
                    tr {
                        th { "Device" }
                        th { "Name" }
                        th { "Board" }
                        th { "Address" }
                        th { "State" }
                        th { "Last started" }
                        th { "Restarts" }
                    }
                    for device in devices.iter() {
                        DeviceRow { key: "{device.id}", device: device.clone(), selected }
                    }
                }
            }
            // The device whose ID was clicked, while it is still in the profile, which Forget would remove it from.
            if let Some(shown) = selected().filter(|shown| devices.iter().any(|device| device.id == shown.device.id)) {
                DeviceCard {
                    key: "{shown.device.id}",
                    device: shown.device.clone(),
                    profile_name: profile_name.clone(),
                    info: shown.info,
                    starts: shown.starts,
                    generation: shown.generation,
                    selected,
                }
            }
            AddDevice { profile_name: profile_name.clone() }
        }
    }
}

/// A device whose card is shown, with what its row has asked it, which the card shows too, rather than asking again,
/// and its row's generation, which Refresh moves on, see DeviceRow.
#[derive(Clone, PartialEq)]
struct Selected {
    device: DeviceProfile,
    info: Resource<anyhow::Result<capture_device::DeviceInfo>>,
    starts: Resource<anyhow::Result<capture_device::Starts>>,
    generation: Signal<u32>,
}

/// One adopted device's row in the table, which asks it what it is, and its starts, now and every HEARTBEAT, and
/// summarises them. Its ID shows its card, see DeviceCard, in place of the one shown.
#[component]
fn DeviceRow(device: DeviceProfile, selected: Signal<Option<Selected>>) -> Element {
    let generation = use_signal(|| 0u32);

    // Devices with firmware from before /api/starts say nothing, so nothing is shown.
    let mut starts = use_resource({
        let device = device.clone();
        move || {
            let device = device.clone();
            generation();
            async move { DeviceClient::for_device(&device)?.starts().await }
        }
    });

    let mut info = use_resource({
        let device = device.clone();
        move || {
            let device = device.clone();
            generation();
            async move {
                let result = async { DeviceClient::for_device(&device)?.info().await }.await;
                if let Err(error) = &result {
                    crate::log_warning!("The capture device {} at {} is not reachable, {error:#}", device.id, device.address);
                }
                result
            }
        }
    });

    // A heartbeat, so that the row shows whether the device is on the network now, not only when the page opened,
    // or Refresh was pressed: what it says it is, over plain HTTP, which costs it little, see DeviceClient::info, and
    // its starts, which show if it restarted. The last of each is shown until the next arrives.
    use_future(move || async move {
        loop {
            tokio::time::sleep(HEARTBEAT).await;
            info.restart();
            starts.restart();
        }
    });

    let is_selected = selected.read().as_ref().is_some_and(|shown| shown.device.id == device.id);
    let show = {
        let device = device.clone();
        move |_| selected.set(Some(Selected { device: device.clone(), info, starts, generation }))
    };

    rsx! {
        tr { class: if is_selected { "selected" },
            td { span { class: "link", onclick: show, "{device.id}" } }
            match &*info.read() {
                None => rsx! {
                    td {} td {}
                    td { "{device.address}" }
                    td { "Connecting..." }
                    td {}
                },
                // The reason is in the log too, see info.
                Some(Err(error)) => rsx! {
                    td {} td {}
                    td { "{device.address}" }
                    td { ErrorText { error: format!("{error:#}"), message: "Not reachable" } }
                    td {}
                },
                Some(Ok(info)) => rsx! {
                    td { {info.hostname.clone().unwrap_or_default()} }
                    td { "{info.board}" }
                    td { "{device.address}" }
                    td {
                        if info.adopted {
                            "Up {up_time(info.up_time_seconds)}"
                        } else {
                            span { class: "invalid", "Not adopted" }
                        }
                    }
                    td {
                        if let Some(reason) = &info.reset_reason {
                            {
                                let (text, trouble) = started_after(reason);
                                rsx! { span { class: if trouble { "invalid" }, "{text}" } }
                            }
                        }
                    }
                },
            }
            td {
                match &*starts.read() {
                    Some(Ok(starts)) if starts.problem.is_some() => rsx! {
                        ErrorText { error: format!("The device could not read its log of starts: {}", starts.problem.clone().unwrap_or_default()), message: "Not known" }
                    },
                    Some(Ok(starts)) => {
                        let restarts = restarts(starts, chrono::Utc::now());
                        rsx! {
                            "{restarts.count} in {RESTART_DAYS} days"
                            for (i, (kind, trouble)) in restarts.kinds.iter().enumerate() {
                                {if i == 0 { ": " } else { ", " }}
                                span { class: if *trouble { "invalid" }, "{kind}" }
                            }
                        }
                    }
                    _ => rsx! {},
                }
            }
        }
    }
}

/// One adopted device: what it says it is, as its row asked it, and its buttons.
#[component]
fn DeviceCard(
    device: DeviceProfile,
    profile_name: String,
    info: Resource<anyhow::Result<capture_device::DeviceInfo>>,
    starts: Resource<anyhow::Result<capture_device::Starts>>,
    generation: Signal<u32>,
    selected: Signal<Option<Selected>>,
) -> Element {
    let mut settings = use_context::<Signal<ProfileSettings>>();
    let mut generation = generation;
    let mut selected = selected;
    let mut status = use_signal(|| None::<Result<String, String>>);
    let mut busy = use_signal(|| false);
    let mut show_settings = use_signal(|| false);
    // The list of the device's starts, see restarts.
    let mut show_starts = use_signal(|| false);
    // Release or Forget waiting for Yes or No.
    let mut confirming = use_signal(|| None::<&'static str>);

    let remove = {
        let device = device.clone();
        let profile_name = profile_name.clone();
        move || {
            let devices: Vec<DeviceProfile> = settings.read().capture_devices.iter().filter(|other| other.id != device.id).cloned().collect();
            if let Err(error) = save_devices(&profile_name, &mut settings, devices) {
                status.set(Some(Err(error)));
            } else {
                selected.set(None);
            }
        }
    };

    let restart = {
        let device = device.clone();
        move |_| {
            let device = device.clone();
            spawn(async move {
                busy.set(true);
                // The last action's message, which this replaces.
                status.set(None);
                let result = async { DeviceClient::for_device(&device)?.restart().await }.await;
                status.set(Some(match result {
                    Ok(()) => Ok("Restarting, it takes about 20 seconds".to_string()),
                    Err(error) => Err(format!("Unable to restart it, {error:#}")),
                }));
                busy.set(false);
            });
        }
    };

    let confirm = {
        let device = device.clone();
        let remove = remove.clone();
        move |_| {
            let action = confirming.take();
            let device = device.clone();
            let mut remove = remove.clone();
            spawn(async move {
                if action == Some("release") {
                    busy.set(true);
                    // The last action's message, which this replaces.
                    status.set(None);
                    let result = async { DeviceClient::for_device(&device)?.release().await }.await;
                    busy.set(false);
                    if let Err(error) = result {
                        status.set(Some(Err(format!("Unable to release it, {error:#}. If it cannot be reached, Forget it, then hold its BOOT button for 5 seconds."))));
                        return;
                    }
                    crate::log_info!("Released the capture device {}", device.id);
                }
                remove();
            });
        }
    };

    rsx! {
        div { class: "device-card",
            h2 { "{device.id}" }
            table { class: "device-details",
                tr { th { "Address" } td { "{device.address}" } }
                match &*info.read() {
                    None => rsx! { tr { th { "State" } td { "Connecting..." } } },
                    // The reason is in the log too, see info.
                    Some(Err(error)) => rsx! { tr { th { "State" } td { ErrorText { error: format!("{error:#}"), message: "Not reachable" } } } },
                    Some(Ok(info)) => rsx! {
                        if let Some(hostname) = &info.hostname {
                            tr { th { "Name" } td { "{hostname}" } }
                        }
                        tr { th { "Board" } td { "{info.board}" } }
                        if let Some(chip) = &info.chip {
                            tr { th { "Chip" } td { {chip.describe()} } }
                        }
                        tr { th { "Firmware" } td { "{info.firmware.name} {info.firmware.version} on {info.platform.name} {info.platform.version}" } }
                        tr { th { "Running for" } td { {up_time(info.up_time_seconds)} } }
                        if let Some(reason) = &info.reset_reason {
                            {
                                let (text, trouble) = started_after(reason);
                                rsx! {
                                    tr { th { "Last started" } td { class: if trouble { "invalid" }, "{text}" } }
                                    if let Some(crash) = &info.last_crash {
                                        tr { th { "Crash" } td { class: "invalid", "{crash}" } }
                                    }
                                }
                            }
                        }
                        // Where it keeps its records, and how its latest polls went, as it said when the app last read
                        // its records, see KeptOn and Takes.
                        if let Some(status) = capture_device::device_statuses().into_iter().find(|status| status.id == device.id) {
                            if let Some(records_device) = status.device.clone() {
                                tr { th { "Records kept on" } td { crate::views::polling::KeptOn { device: records_device } } }
                            }
                            if !status.polls.is_empty() {
                                tr { th { "Polls" } td { crate::views::polling::Takes { status } } }
                            }
                        }
                        if let Some(Ok(starts)) = &*starts.read() {
                            {
                                let restarts = restarts(starts, chrono::Utc::now());
                                let problem = starts.problem.clone();
                                rsx! { tr { th { "Restarts" } td {
                                    // A device which could not read its log, e.g. older firmware, which kept it on the
                                    // SD card, with no card, gives only its last start, which says nothing of the
                                    // others, so no count is shown, only that they are not known.
                                    if let Some(problem) = problem {
                                        div {
                                            ErrorText {
                                                error: format!("The device could not read its log of starts, so it only knows why it last started, \
                                                    see Last started. Older firmware keeps the log on the SD card, so has none without a card: {problem}"),
                                                message: "Not known",
                                            }
                                        }
                                    } else {
                                        div {
                                            // Shows or hides the list.
                                            span { class: "link", onclick: move |_| show_starts.toggle(),
                                                "{restarts.count} in the last {RESTART_DAYS} days"
                                            }
                                            for (i, (kind, trouble)) in restarts.kinds.iter().enumerate() {
                                                {if i == 0 { ": " } else { ", " }}
                                                span { class: if *trouble { "invalid" }, "{kind}" }
                                            }
                                            if restarts.not_logged > 0 {
                                                ", and {restarts.not_logged} more not logged"
                                            }
                                        }
                                    }
                                    if show_starts() {
                                        table { class: "device-starts",
                                            for (time, count, text, trouble, gap) in restarts.list {
                                                tr { class: if trouble { "invalid" },
                                                    td { "{time}" }
                                                    td { class: "number", {count.map(|count| format!("#{count}")).unwrap_or_default()} }
                                                    td { "{text}" }
                                                }
                                                if gap > 0 {
                                                    tr { td { colspan: 3, class: "derived",
                                                        {format!("{gap} {} not logged", if gap == 1 { "start" } else { "starts" })}
                                                    } }
                                                }
                                            }
                                        }
                                    }
                                } } }
                            }
                        }
                        tr { th { "Features" } td {
                            {info.features.iter().map(|feature| format!("{}{}", feature.label.as_ref().unwrap_or(&feature.name), if feature.enabled { "" } else { " (off)" })).collect::<Vec<_>>().join(", ")}
                        } }
                        if !info.adopted {
                            tr { th { "State" } td { class: "invalid", "No longer adopted, its BOOT button may have been held. Forget it, then adopt it again." } }
                        }
                    },
                }
            }
            BatteryHistory { profile_name: profile_name.clone(), device_id: device.id.clone() }
            div { class: "settings-buttons",
                button { r#type: "button", disabled: busy(), onclick: move |_| show_settings.set(!show_settings()), if show_settings() { "Hide Settings" } else { "Settings" } }
                button { r#type: "button", disabled: busy(), onclick: move |_| { status.set(None); generation += 1; }, "Refresh" }
                // Only while the device answers, as otherwise it cannot be asked.
                button { r#type: "button", disabled: busy() || !matches!(&*info.read(), Some(Ok(_))), onclick: restart, "Restart" }
                button { r#type: "button", disabled: busy(), onclick: move |_| confirming.set(Some("release")), "Release" }
                button { r#type: "button", disabled: busy(), onclick: move |_| confirming.set(Some("forget")), "Forget" }
            }
            match confirming() {
                Some(action) => rsx! {
                    div { class: "settings-buttons",
                        span {
                            if action == "release" {
                                "Release the device, so that its own /config page manages it again, and remove it from this profile?"
                            } else {
                                "Remove the device from this profile without releasing it? It stays adopted until its BOOT button is held for 5 seconds."
                            }
                        }
                        button { r#type: "button", onclick: confirm, "Yes" }
                        button { r#type: "button", onclick: move |_| confirming.set(None), "No" }
                    }
                },
                None => rsx! {},
            }
            {status_line(status())}
            if show_settings() {
                DeviceSettings { device: device.clone() }
            }
        }
    }
}

/// Finds a device by its address and adopts it with the code on its screen.
#[component]
fn AddDevice(profile_name: String) -> Element {
    let mut settings = use_context::<Signal<ProfileSettings>>();
    let poll_statuses = try_use_context::<crate::polling::PollStatuses>();
    let mut address = use_signal(String::new);
    let mut found = use_signal(|| None::<FoundDevice>);
    let mut pairing = use_signal(|| false);
    let mut code = use_signal(String::new);
    let mut status = use_signal(|| None::<Result<String, String>>);
    let mut busy = use_signal(|| false);

    let find = move || {
        spawn(async move {
            busy.set(true);
            status.set(None);
            found.set(None);
            pairing.set(false);
            match capture_device::find(&address()).await {
                Ok(device) => {
                    if settings.read().capture_devices.iter().any(|known| known.id == device.info.id) {
                        status.set(Some(Err(format!("{} is already in this profile", device.info.id))));
                    } else if device.info.adopted {
                        status.set(Some(Err("Another app, or another profile, has adopted it. To adopt it here, hold its BOOT button for 5 seconds to release it, then Find it again.".to_string())));
                        found.set(Some(device));
                    } else {
                        found.set(Some(device));
                    }
                }
                Err(error) => status.set(Some(Err(format!("{error:#}")))),
            }
            busy.set(false);
        });
    };

    let start = move |_| {
        spawn(async move {
            let Some(device) = found() else { return };
            busy.set(true);
            // The last action's message, which this replaces.
            status.set(None);
            match capture_device::start_pairing(&device).await {
                Ok(time) => {
                    pairing.set(true);
                    code.set(String::new());
                    status.set(Some(Ok(format!("The device is showing a code, for {} minutes", time.as_secs() / 60))));
                }
                Err(error) => status.set(Some(Err(format!("Unable to start pairing, {error:#}")))),
            }
            busy.set(false);
        });
    };

    let adopt = move || {
        let profile_name = profile_name.clone();
        spawn(async move {
            let Some(device) = found() else { return };
            busy.set(true);
            // The last action's message, which this replaces.
            status.set(None);
            match capture_device::adopt(&device, &code()).await {
                Ok(adopted) => {
                    let mut devices = settings.read().capture_devices.clone();
                    devices.retain(|other| other.id != adopted.id);
                    devices.push(adopted);
                    match save_devices(&profile_name, &mut settings, devices) {
                        Ok(()) => {
                            // Its card, which appears above, says so.
                            status.set(None);
                            if let Some(statuses) = poll_statuses {
                                for module_id in READING_MODULES {
                                    crate::polling::poll_now(statuses, module_id);
                                }
                            }
                            found.set(None);
                            pairing.set(false);
                            address.set(String::new());
                        }
                        Err(error) => status.set(Some(Err(format!("The device was adopted, but {error}. Hold its BOOT button for 5 seconds and adopt it again.")))),
                    }
                }
                Err(error) => status.set(Some(Err(format!("Not adopted, {error:#}")))),
            }
            busy.set(false);
        });
    };

    rsx! {
        h2 { "Add a Capture Device" }
        p { class: "note", "Enter the device's IP address, shown on its /config page or by your router." }
        div { class: "settings-buttons",
            input {
                r#type: "text",
                placeholder: "e.g. 192.168.1.50",
                value: "{address}",
                oninput: move |event| address.set(event.value()),
                // Enter is the same as Find.
                onkeydown: move |event: KeyboardEvent| {
                    if event.key() == Key::Enter && !busy() && !address().trim().is_empty() {
                        find();
                    }
                },
            }
            button { r#type: "button", disabled: busy() || address().trim().is_empty(), onclick: move |_| find(), "Find" }
        }
        if let Some(device) = found() {
            table { class: "device-details",
                tr { th { "Device" } td { "{device.info.id}" } }
                tr { th { "Board" } td { "{device.info.board}" } }
                if let Some(chip) = &device.info.chip {
                    tr { th { "Chip" } td { {chip.describe()} } }
                }
                tr { th { "Firmware" } td { "{device.info.firmware.name} {device.info.firmware.version}" } }
                tr { th { "Certificate" } td { {device.fingerprint[..8].to_string()} } }
            }
            if !device.info.adopted {
                if pairing() {
                    p { class: "note",
                        {format!("Type the code on the device's screen. The screen also shows Certificate {}: if it shows something else, something else may be in between, so do not adopt it.", &device.fingerprint[..8])}
                    }
                    div { class: "settings-buttons",
                        input {
                            r#type: "text",
                            placeholder: "123 456",
                            value: "{code}",
                            oninput: move |event| code.set(event.value()),
                            // Enter is the same as Adopt.
                            onkeydown: {
                                let adopt = adopt.clone();
                                move |event: KeyboardEvent| {
                                    if event.key() == Key::Enter && !busy() && !code().trim().is_empty() {
                                        adopt();
                                    }
                                }
                            },
                        }
                        button { r#type: "button", disabled: busy() || code().trim().is_empty(), onclick: { let adopt = adopt.clone(); move |_| adopt() }, "Adopt" }
                        button { r#type: "button", disabled: busy(), onclick: start, "New Code" }
                    }
                } else {
                    div { class: "settings-buttons",
                        button { r#type: "button", disabled: busy(), onclick: start, "Show Pairing Code" }
                    }
                }
            }
        }
        {status_line(status())}
    }
}

/// The device's settings, a form for each feature, from what /api/config says about them.
#[component]
fn DeviceSettings(device: DeviceProfile) -> Element {
    let mut generation = use_signal(|| 0u32);
    let mut edits = use_signal(Edits::new);
    let mut enabled_edits = use_signal(BTreeMap::<String, bool>::new);
    let mut status = use_signal(|| None::<Result<String, String>>);
    let mut busy = use_signal(|| false);
    let mut restart_needed = use_signal(|| false);

    let config = use_resource({
        let device = device.clone();
        move || {
            let device = device.clone();
            generation();
            async move { DeviceClient::for_device(&device)?.config().await }
        }
    });

    let save = {
        let device = device.clone();
        move |_| {
            let mut features = Map::new();
            for (feature, enabled) in enabled_edits.read().iter() {
                features.insert(feature.clone(), json!({ "enabled": enabled }));
            }
            for ((feature, name), value) in edits.read().iter() {
                let entry = features.entry(feature.clone()).or_insert_with(|| json!({}));
                entry["settings"][name] = value.clone();
            }
            if features.is_empty() {
                status.set(Some(Ok("Nothing has changed".to_string())));
                return;
            }
            let device = device.clone();
            spawn(async move {
                busy.set(true);
                // The last action's message, which this replaces.
                status.set(None);
                let update = json!({ "features": features });
                let result = async { DeviceClient::for_device(&device)?.update_config(&update).await }.await;
                match result {
                    Ok(_) => {
                        edits.write().clear();
                        enabled_edits.write().clear();
                        restart_needed.set(true);
                        status.set(Some(Ok("Saved, they take effect when the device restarts".to_string())));
                        generation += 1;
                    }
                    Err(error) => status.set(Some(Err(format!("Not saved, {error:#}")))),
                }
                busy.set(false);
            });
        }
    };

    let restart = {
        let device = device.clone();
        move |_| {
            let device = device.clone();
            spawn(async move {
                busy.set(true);
                // The last action's message, which this replaces.
                status.set(None);
                let result = async { DeviceClient::for_device(&device)?.restart().await }.await;
                match result {
                    Ok(()) => {
                        restart_needed.set(false);
                        status.set(Some(Ok("Restarting, it takes about 20 seconds".to_string())));
                    }
                    Err(error) => status.set(Some(Err(format!("Unable to restart it, {error:#}")))),
                }
                busy.set(false);
            });
        }
    };

    rsx! {
        div { class: "device-settings",
            match &*config.read() {
                None => rsx! { p { "Reading the settings..." } },
                Some(Err(error)) => rsx! { p { class: "invalid", {format!("Unable to read the settings, {error:#}")} } },
                Some(Ok(json)) => rsx! {
                    for feature in json["features"].as_array().cloned().unwrap_or_default() {
                        FeatureSettings { key: "{feature[\"name\"]}", feature, edits, enabled_edits, device_id: device.id.clone() }
                    }
                },
            }
            div { class: "settings-buttons",
                button { r#type: "button", disabled: busy(), onclick: save, "Save" }
                button { r#type: "button", disabled: busy(), onclick: move |_| { edits.write().clear(); enabled_edits.write().clear(); status.set(None); }, "Undo Changes" }
                // Saved settings take effect when the device restarts.
                button { r#type: "button", disabled: busy(), onclick: restart, if restart_needed() { "Restart Now" } else { "Restart" } }
            }
            {status_line(status())}
        }
    }
}

/// One feature's settings, with whether it is enabled.
#[component]
fn FeatureSettings(feature: Value, edits: Signal<Edits>, enabled_edits: Signal<BTreeMap<String, bool>>, device_id: String) -> Element {
    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let profile_name = context_signal.read().as_ref().map(|context| context.profile.active_profile.name.clone()).unwrap_or_default();
    let name = feature["name"].as_str().unwrap_or_default().to_string();
    let fillable = FROM_PROFILE.iter().any(|(feature, _, _)| *feature == name);
    // What the device says to show it as, e.g. Tesla Powerwall, otherwise its name.
    let title = feature["label"].as_str().map(String::from).unwrap_or_else(|| name.clone());
    let required = feature["enabled"] == json!("required");
    let enabled = enabled_edits.read().get(&name).copied().unwrap_or_else(|| feature["enabled"] == json!(true));
    let settings = feature["settings"].as_array().cloned().unwrap_or_default();

    rsx! {
        fieldset { class: "device-feature",
            legend { "{title}" }
            if required {
                p { class: "note", "Always enabled" }
            } else {
                label { class: "device-setting",
                    span { "Enabled" }
                    input {
                        r#type: "checkbox",
                        checked: enabled,
                        onchange: {
                            let name = name.clone();
                            move |event: FormEvent| { enabled_edits.write().insert(name.clone(), event.checked()); }
                        },
                    }
                }
            }
            if fillable {
                div { class: "settings-buttons",
                    button {
                        r#type: "button",
                        title: "Fill in the settings this profile's modules know, e.g. the API key, to check, then Save",
                        onclick: {
                            let (name, profile_name) = (name.clone(), profile_name.clone());
                            move |_| edits.write().extend(from_profile(&profile_name, &name))
                        },
                        "Fill in from this profile"
                    }
                }
            }
            for setting in settings {
                SettingField { key: "{setting[\"name\"]}", feature: name.clone(), setting: setting.clone(), edits }
                // The temperature sensor's offset is worked out from what a thermometer beside it says, see Calibrate.
                if setting["name"] == json!(TEMPERATURE_OFFSET) {
                    Calibrate { profile_name: profile_name.clone(), device_id: device_id.clone(), feature: name.clone(), setting, edits }
                }
            }
        }
    }
}

/// One setting, with an input for its type. A secret is never shown, only whether it is set: typing replaces it, and
/// Clear Password clears an optional one.
#[component]
fn SettingField(feature: String, setting: Value, edits: Signal<Edits>) -> Element {
    let name = setting["name"].as_str().unwrap_or_default().to_string();
    // What the device says to show it as, e.g. WiFi Password, otherwise its name.
    let label = setting["label"].as_str().map(String::from).unwrap_or_else(|| name.clone());
    let kind = setting["type"].as_str().unwrap_or_default().to_string();
    let required = setting["required"].as_bool().unwrap_or_default();
    let secret = setting["secret"].as_bool().unwrap_or_default();
    let key = (feature, name.clone());
    let edited = edits.read().get(&key).cloned();
    let value = edited.clone().unwrap_or_else(|| setting["value"].clone());
    let text = match &value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        value => value.to_string(),
    };
    // Sets the edit to what the input says, removing it if that is what the device has, or, for a secret, if it is
    // empty, which keeps what the device has.
    let set = {
        let key = key.clone();
        let original = setting["value"].clone();
        move |value: Value| {
            if value == original || (secret && value == json!("")) {
                edits.write().remove(&key);
            } else {
                edits.write().insert(key.clone(), value);
            }
        }
    };
    // Empty clears an optional setting, as on the device's /config page.
    let text_value = move |text: String| if text.trim().is_empty() { Value::Null } else { json!(text) };
    let max_length = setting["maxLength"].as_u64().map(|len| len.to_string()).unwrap_or_default();
    let changed = edited.is_some();

    rsx! {
        label { class: if changed { "device-setting changed" } else { "device-setting" },
            span { "{label}" }
            match kind.as_str() {
                "bool" => rsx! {
                    input {
                        r#type: "checkbox",
                        checked: value.as_bool().unwrap_or_default(),
                        onchange: { let mut set = set.clone(); move |event: FormEvent| set(json!(event.checked())) },
                    }
                },
                "int32" | "int64" => rsx! {
                    input {
                        r#type: "number",
                        value: "{text}",
                        placeholder: if required { "" } else { "Default" },
                        oninput: {
                            let mut set = set.clone();
                            move |event: FormEvent| set(event.value().trim().parse::<i64>().map(|number| json!(number)).unwrap_or(Value::Null))
                        },
                    }
                },
                "timeZone" => rsx! {
                    select {
                        onchange: { let mut set = set.clone(); move |event: FormEvent| set(json!(event.value())) },
                        for option in setting["options"].as_array().cloned().unwrap_or_default() {
                            option {
                                value: option.as_str().unwrap_or_default(),
                                selected: option == value,
                                {option.as_str().unwrap_or_default().to_string()}
                            }
                        }
                    }
                },
                "color" => rsx! {
                    input {
                        r#type: "color",
                        value: if text.is_empty() { "#000000".to_string() } else { text.to_lowercase() },
                        oninput: { let mut set = set.clone(); move |event: FormEvent| set(json!(event.value().to_uppercase())) },
                    }
                    if !required && !value.is_null() {
                        button { r#type: "button", onclick: { let mut set = set.clone(); move |_| set(Value::Null) }, "Default" }
                    }
                },
                _ if secret => {
                    let is_set = setting["isSet"].as_bool().unwrap_or_default();
                    let cleared = edited == Some(Value::Null);
                    let placeholder = if cleared { "Cleared" } else if is_set { "Set, leave empty to keep" } else { "Not set" };
                    rsx! {
                        input {
                            r#type: "password",
                            maxlength: "{max_length}",
                            placeholder,
                            value: if cleared { String::new() } else { text.clone() },
                            oninput: { let mut set = set.clone(); move |event: FormEvent| set(json!(event.value())) },
                        }
                        if is_set && !required {
                            button { r#type: "button", onclick: { let mut set = set.clone(); move |_| set(Value::Null) }, "Clear Password" }
                        }
                    }
                },
                _ => rsx! {
                    input {
                        r#type: "text",
                        maxlength: "{max_length}",
                        value: "{text}",
                        placeholder: if required { "" } else { "Default" },
                        oninput: { let mut set = set.clone(); move |event: FormEvent| set(text_value(event.value())) },
                    }
                    if let Some(description) = setting["description"].as_str() {
                        span { class: "note", "{description}" }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_counts_the_last_week_by_kind_and_those_not_logged() {
        let start = |time: &str, reason: &str, count| capture_device::Start { time: time.to_string(), reason: reason.to_string(), count: Some(count) };
        let starts = capture_device::Starts {
            count: Some(9),
            starts: vec![
                start("2026-10-20T10:00:00Z", "PowerOn", 3),
                start("2026-10-29T18:36:10Z", "PowerOn", 5),
                start("2026-10-30T06:00:33Z", "Panic", 6),
                start("2026-10-30T06:50:00Z", "Software", 9),
            ],
            problem: None,
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-30T07:00:00Z").unwrap().to_utc();
        let restarts = restarts(&starts, now);
        assert_eq!(restarts.count, 3);
        assert_eq!(restarts.kinds, vec![("1 crash".to_string(), true), ("1 restart".to_string(), false), ("1 switch-on".to_string(), false)]);
        assert_eq!(restarts.not_logged, 2);
        // Newest first, with the 2 not logged before the last.
        assert_eq!(restarts.list.iter().map(|(_, count, _, trouble, gap)| (*count, *trouble, *gap)).collect::<Vec<_>>(),
            vec![(Some(9), false, 2), (Some(6), true, 0), (Some(5), false, 0)]);
    }
}

/// How many days of a device's battery the Devices page shows.
const BATTERY_DAYS: i64 = 2;

/// A device's battery's voltage over the last BATTERY_DAYS days, as it gave it with its temperature and humidity, see
/// climate::BATTERY, as of the app's last read of it, nothing if it has none, e.g. a board without a battery.
#[component]
fn BatteryHistory(profile_name: String, device_id: String) -> Element {
    let tz = time_tz::system::get_timezone().unwrap_or(time_tz::timezones::db::UTC);
    let now = crate::clock::now_utc();
    let from = now - time::Duration::days(BATTERY_DAYS);
    let periods = crate::climate::dir(&profile_name)
        .map(|dir| crate::climate::load_series(&dir, &device_id, &crate::climate::BATTERY, from, now))
        .unwrap_or_default();
    let Some(latest) = periods.last().map(|period| period.value) else {
        return rsx! {};
    };
    rsx! {
        h3 { "Battery, {latest:.2} V" }
        div { class: "device-battery",
            {super::climate::series_graph(&crate::climate::BATTERY, &[("Battery", periods.as_slice())], from.unix_timestamp(), now.unix_timestamp(), tz)}
        }
    }
}

/// The name of the temperature sensor's offset setting, in tenths of a degree, see the platform's
/// features/temperature_humidity.rs, which Calibrate works out.
const TEMPERATURE_OFFSET: &str = "offset_tenths";

/// Works out the temperature sensor's offset from what a thermometer beside the device says, typed in, and fills it in
/// to Save: the device's latest temperature, as the app last read it, see climate.rs, less the offset it was taken with,
/// setting's value on the device, is what the sensor itself read, and the offset is the thermometer's less that. To
/// within a tenth, as the readings are kept to tenths, and as of the app's last read, a few minutes ago at most.
#[component]
fn Calibrate(profile_name: String, device_id: String, feature: String, setting: Value, edits: Signal<Edits>) -> Element {
    let mut actual = use_signal(String::new);
    let mut note = use_signal(|| None::<Result<String, String>>);
    let now = crate::clock::now_utc();
    let latest = crate::climate::dir(&profile_name).ok()
        .map(|dir| crate::climate::load_series(&dir, &device_id, &crate::climate::SERIES[0], now - time::Duration::minutes(30), now))
        .and_then(|periods| periods.last().map(|period| period.value));
    let offset = setting["value"].as_f64().unwrap_or(0.0) / 10.0;
    let work_out = move |_| {
        let Ok(thermometer) = actual.read().trim().parse::<f64>() else {
            note.set(Some(Err(String::from("Type the temperature the thermometer says, e.g. 24.8"))));
            return;
        };
        let Some(latest) = latest else {
            note.set(Some(Err(String::from("The app has no temperature from this device in the last half hour"))));
            return;
        };
        let sensor = latest - offset;
        let tenths = ((thermometer - sensor) * 10.0).round() as i64;
        edits.write().insert((feature.clone(), String::from(TEMPERATURE_OFFSET)), json!(tenths));
        note.set(Some(Ok(format!("The sensor reads {sensor:.1} °C, so the offset is {tenths}. Save to use it."))));
    };
    rsx! {
        label { class: "device-setting",
            span { "Actual temperature now (°C), from a thermometer beside it" }
            input {
                r#type: "number",
                step: "0.1",
                placeholder: latest.map(|latest| format!("shown now {latest:.1}")).unwrap_or_default(),
                value: "{actual}",
                oninput: move |event| actual.set(event.value()),
            }
            button { r#type: "button", onclick: work_out, "Work out offset" }
        }
        match note() {
            Some(Ok(text)) => rsx! { p { class: "note", "{text}" } },
            Some(Err(text)) => rsx! { p { class: "invalid", "{text}" } },
            None => rsx! {},
        }
    }
}
