//! The settings of the active profile which are the app's rather than a module's, see profile::Settings: when the log
//! is started again, and whether the log panel is shown.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::AppContext;
use crate::logging::{self, LogRetention};
use crate::profile::Settings as ProfileSettings;

/// The defaults of each kind of retention, offered when it is chosen.
const DEFAULT_HOURS: u32 = logging::DEFAULT_HOURS;
const DEFAULT_MEGABYTES: u32 = 10;

#[component]
pub fn Settings() -> Element {
    let context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let mut settings = use_context::<Signal<ProfileSettings>>();
    let profile_name = context_signal
        .read()
        .as_ref()
        .map(|context| context.profile.active_profile.name.clone())
        .unwrap_or_default();

    // The settings being edited, until they are saved.
    let mut retention = use_signal(|| settings.read().log.retention);
    let mut hours = use_signal(|| match settings.read().log.retention {
        LogRetention::Hours { hours } => hours,
        _ => DEFAULT_HOURS,
    });
    let mut megabytes = use_signal(|| match settings.read().log.retention {
        LogRetention::Size { megabytes } => megabytes,
        _ => DEFAULT_MEGABYTES,
    });
    let mut status = use_signal(|| None::<Result<String, String>>);

    let chosen = |kind: &str| match (kind, retention()) {
        ("eachRun", LogRetention::EachRun)
        | ("hours", LogRetention::Hours { .. })
        | ("size", LogRetention::Size { .. }) => true,
        _ => false,
    };
    let log_path = logging::log_path(&profile_name)
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    let save = {
        let profile_name = profile_name.clone();
        move |_| {
            let mut new_settings = settings.read().clone();
            new_settings.log.retention = match retention() {
                LogRetention::EachRun => LogRetention::EachRun,
                LogRetention::Hours { .. } => LogRetention::Hours { hours: hours().max(1) },
                LogRetention::Size { .. } => LogRetention::Size {
                    megabytes: megabytes().max(1),
                },
            };
            match crate::profile::update_settings(&profile_name, &new_settings) {
                Ok(()) => {
                    logging::set_retention(new_settings.log.retention);
                    crate::log_info!("Saved the settings of profile {profile_name}");
                    settings.set(new_settings);
                    status.set(Some(Ok(String::from("Saved"))));
                }
                Err(error) => {
                    crate::log_error!("Failed to save the settings of profile {profile_name}: {error}");
                    status.set(Some(Err(format!("Failed to save the settings: {error}"))));
                }
            }
        }
    };

    rsx! {
        div { class: "settings-page",
            h1 { "Settings" }
            p { class: "note", "The settings of profile {profile_name}." }

            h2 { "Log" }
            p { class: "note", "The log records the important things the app does, e.g. the data it fetched and anything which failed, including what a scheduled fetch does. It is kept in {log_path}." }
            p { class: "note", "Rather than old entries being removed, the log is started again from time to time, and the previous log is kept in the same place with .old added to its name, replacing the one before." }
            div { class: "settings-group",
                label {
                    input {
                        r#type: "radio",
                        name: "log-keep",
                        checked: chosen("hours"),
                        onchange: move |_| retention.set(LogRetention::Hours { hours: hours() }),
                    }
                    " Keep at least the last "
                    input {
                        class: "settings-number",
                        r#type: "number",
                        min: "1",
                        value: "{hours}",
                        disabled: !chosen("hours"),
                        oninput: move |event| if let Ok(value) = event.value().parse() { hours.set(value) },
                    }
                    " hours"
                }
                label {
                    input {
                        r#type: "radio",
                        name: "log-keep",
                        checked: chosen("eachRun"),
                        onchange: move |_| retention.set(LogRetention::EachRun),
                    }
                    " Start the log again each time the app is opened"
                }
                label {
                    input {
                        r#type: "radio",
                        name: "log-keep",
                        checked: chosen("size"),
                        onchange: move |_| retention.set(LogRetention::Size { megabytes: megabytes() }),
                    }
                    " Keep up to "
                    input {
                        class: "settings-number",
                        r#type: "number",
                        min: "1",
                        value: "{megabytes}",
                        disabled: !chosen("size"),
                        oninput: move |event| if let Ok(value) = event.value().parse() { megabytes.set(value) },
                    }
                    " MB in all"
                }
            }
            p { class: "note", "Starting the log again each time the app is opened takes effect the next time it is opened." }

            div { class: "settings-buttons",
                button { r#type: "button", onclick: save, "Save" }
                match status() {
                    Some(Ok(message)) => rsx! { span { class: "ok", "{message}" } },
                    Some(Err(message)) => rsx! { span { class: "invalid", "{message}" } },
                    None => rsx! {},
                }
            }
        }
    }
}
