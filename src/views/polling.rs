use dioxus::prelude::*;
use time::macros::format_description;
use time_tz::OffsetDateTimeExt;

use crate::capture_device::DeviceStatus;
use crate::components::error_text::ErrorText;
use crate::polling::{PollState, PollStatuses, poll_now, set_paused};

/// The time in the computer's time zone, or UTC if it can not be found, e.g. 14:32:50
fn shown_time(time: Option<time::OffsetDateTime>) -> String {
    let format = format_description!("[hour]:[minute]:[second]");
    match (time, time_tz::system::get_timezone().ok()) {
        (Some(time), Some(tz)) => time.to_timezone(tz).format(&format).unwrap_or_default(),
        (Some(time), None) => time.format(&format).unwrap_or_default(),
        (None, _) => String::new(),
    }
}

/// When a device said it started, an RFC 3339 time, in the computer's time zone, e.g. 1 Nov 16:25.
fn shown_date_time(time: &str) -> String {
    let format = format_description!("[day padding:none] [month repr:short] [hour]:[minute]");
    let Ok(time) = time::OffsetDateTime::parse(time, &time::format_description::well_known::Rfc3339) else {
        return time.to_string();
    };
    match time_tz::system::get_timezone().ok() {
        Some(tz) => time.to_timezone(tz).format(&format).unwrap_or_default(),
        None => time.format(&format).unwrap_or_default(),
    }
}

/// What the app's polling of each module is doing, see polling.rs, and how reading the capture devices which poll
/// for it is going, see capture_device.rs.
#[component]
pub fn PollingPage() -> Element {
    let statuses = use_context::<PollStatuses>();
    let rows: Vec<_> = statuses.read().iter().map(|(module_id, status)| (module_id.clone(), status.clone())).collect();
    // They change as the modules are polled, which is when statuses changes, so reading it above shows them afresh.
    let devices = crate::capture_device::device_statuses();

    rsx! {
        h1 { "Polling" }
        p {
            "While the app is open it polls the modules whose data nothing else keeps, such as the Tesla gateway and the zappi's status. Where capture devices take a module's data, the app reads it from them instead, and does not poll it itself. Each poll and read is in the log too."
        }
        if rows.is_empty() {
            p { "There is nothing to poll in this profile." }
        } else {
            PollingTable { rows, statuses }
        }
        if !devices.is_empty() {
            h2 { "Capture Devices" }
            DevicesTable { devices }
        }
    }
}

/// How reading each capture device is going.
#[component]
fn DevicesTable(devices: Vec<DeviceStatus>) -> Element {
    rsx! {
        table { class: "display",
            tr {
                th { "Device" }
                th { "Takes" }
                th { "Last Read" }
                th { "Result" }
                th { "Kept On" }
                th { "Started" }
                th { "Free Memory" }
            }
            for status in devices {
                tr {
                    td { title: "ID {status.id} at {status.address}", {status.name.clone().unwrap_or_else(|| status.id.clone())} }
                    td { {status.takes.iter().cloned().collect::<Vec<_>>().join(", ")} }
                    td { {shown_time(status.last_read)} }
                    td {
                        match &status.last_error {
                            Some((time, error)) => rsx! { ErrorText { error: error.clone(), message: format!("Not read at {}", shown_time(Some(*time))) } },
                            None => rsx! { "Read {status.read} records, {status.saved} new" },
                        }
                    }
                    td {
                        if let Some(device) = &status.device {
                            match (device.storage.as_deref(), &device.storage_problem) {
                                (Some("sd"), _) => rsx! { "SD card" },
                                (Some("ram"), problem) => {
                                    let kept = match (device.records_in_ram, device.ram_capacity) {
                                        (Some(records), Some(capacity)) => format!(", {records} of {capacity}"),
                                        _ => String::new(),
                                    };
                                    match problem {
                                        Some(problem) => rsx! { span { title: "Not on an SD card, {problem}", "RAM{kept}" } },
                                        None => rsx! { "RAM{kept}" },
                                    }
                                },
                                (Some(other), _) => rsx! { "{other}" },
                                (None, _) => rsx! {},
                            }
                        }
                    }
                    td {
                        if let Some(device) = &status.device {
                            if let Some(boot_time) = &device.boot_time {
                                {shown_date_time(boot_time)}
                            }
                            if let Some(reason) = &device.reset_reason {
                                {
                                    let (text, trouble) = crate::views::devices::started_after(reason);
                                    // Why it crashed, if it knows, as the tooltip.
                                    match &device.last_crash {
                                        Some(crash) => rsx! { ", " span { class: "invalid", title: "{crash}", "{text.to_lowercase()}" } },
                                        None => rsx! { ", " span { class: if trouble { "invalid" }, "{text.to_lowercase()}" } },
                                    }
                                }
                            }
                        }
                    }
                    td { class: "number",
                        if let Some(device) = &status.device {
                            if let Some(free) = device.free_heap {
                                {
                                    let mut details = Vec::new();
                                    if let Some(least) = device.minimum_free_heap {
                                        details.push(format!("The least since it started was {} KB", least / 1024));
                                    }
                                    if let Some(largest) = device.largest_free_block {
                                        details.push(format!("the largest free block {} KB", largest / 1024));
                                    }
                                    if let Some(failed) = device.failed_allocations.filter(|failed| *failed > 0) {
                                        details.push(format!("{failed} allocations have failed"));
                                    }
                                    let low = device.failed_allocations.is_some_and(|failed| failed > 0);
                                    if details.is_empty() {
                                        rsx! { "{free / 1024} KB" }
                                    } else {
                                        rsx! { span { class: if low { "invalid" }, title: "{details.join(\", \")}", "{free / 1024} KB" } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PollingTable(rows: Vec<(String, crate::polling::PollStatus)>, statuses: PollStatuses) -> Element {
    rsx! {
        table { class: "display",
            tr {
                th { "Module" }
                th { "From" }
                th { "State" }
                th { "Every" }
                th { "Last Poll" }
                th { "Result" }
                th { "Next Poll" }
                th { "Polls" }
                th { "Failures" }
                th {}
            }
            for (module_id , status) in rows {
                tr {
                    td { "{module_id}" }
                    td {
                        // Known once it has been polled, as that is when the devices say what they take.
                        if status.polls > 0 {
                            {
                                let devices = crate::capture_device::devices_taking(&module_id);
                                if devices.is_empty() { "This computer".to_string() } else { devices.join(", ") }
                            }
                        }
                    }
                    td {
                        {
                            match &status.state {
                                PollState::Starting => "Starting".to_string(),
                                PollState::NotPolled(why) => format!("Not polled: {why}"),
                                PollState::Waiting => "Waiting".to_string(),
                                PollState::Polling => "Polling...".to_string(),
                                PollState::Paused => "Paused".to_string(),
                            }
                        }
                    }
                    td { {status.interval.map(|interval| format!("{} min", interval.as_secs() / 60)).unwrap_or_default()} }
                    td { {shown_time(status.last_time)} }
                    td {
                        match &status.last_result {
                            Some(Ok(summary)) => rsx! { "{summary}" },
                            Some(Err(error)) => rsx! { ErrorText { error: error.clone() } },
                            None => rsx! {},
                        }
                    }
                    td {
                        if status.state == PollState::Waiting {
                            {shown_time(status.next_time)}
                        }
                    }
                    td { class: "number", "{status.polls}" }
                    td { class: "number", "{status.failures}" }
                    td { class: "poll-buttons",
                        // Shown, but disabled, while polling, so that the buttons do not jump about.
                        if matches!(status.state, PollState::Waiting | PollState::Paused | PollState::Polling) {
                            button {
                                disabled: status.state == PollState::Polling,
                                onclick: {
                                    let module_id = module_id.clone();
                                    move |_| poll_now(statuses, &module_id)
                                },
                                "Poll Now"
                            }
                        }
                        if matches!(status.state, PollState::Waiting | PollState::Polling) {
                            button {
                                disabled: status.state == PollState::Polling,
                                onclick: {
                                    let module_id = module_id.clone();
                                    move |_| set_paused(statuses, &module_id, true)
                                },
                                "Pause"
                            }
                        }
                        if status.state == PollState::Paused {
                            button {
                                onclick: {
                                    let module_id = module_id.clone();
                                    move |_| set_paused(statuses, &module_id, false)
                                },
                                "Resume"
                            }
                        }
                    }
                }
            }
        }
    }
}
