use std::{collections::BTreeMap, sync::Arc};

use crate::{
    AppContext, ModuleRegistrations, PageInfo, UpgradeRequested, components::app::Route, profile::CURRENT_VERSION,
};

use super::polling::PollingPage;
use super::sidebar::SidebarLayout;
use dioxus::prelude::*;

include!(concat!(env!("OUT_DIR"), "/crate_info.rs"));

const APP_ICON: Asset = asset!("/assets/app-icon.png");

/// The Home page component that will be rendered when the current route is `[Route::Home]`
#[component]
pub fn Home(// xid: i32,
    // modules_signal: ModuleRegistrations
) -> Element {
    let context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let opt_context = &*context_signal.read();
    let context = opt_context.as_ref().unwrap();
    let module_registrations = use_context::<ModuleRegistrations>();
    // By name, so that they are always in the same order.
    let mut modules = BTreeMap::new();
    let mut upgrade_requested = use_context::<Signal<UpgradeRequested>>();
    // The page of Home which is shown, see HOME_PAGES: Electricity if the profile has any modules, otherwise Modules,
    // to choose some.
    let has_modules = !context.profile.active_profile.modules.is_empty();
    let mut page = use_signal(|| if has_modules { "electricity" } else { "modules" });
    // The help follows the page shown, see the Navbar.
    let home_help = try_use_context::<crate::help::HomeHelpTopic>();
    use_effect(move || {
        let topic = match page() {
            "electricity" => "electricity.md",
            "site" => "site.md",
            "live" => "live.md",
            "devices" => "devices.md",
            "climate" => "climate.md",
            _ => "home.md",
        };
        if let Some(mut help) = home_help
            && *help.0.peek() != topic
        {
            help.0.set(topic);
        }
    });

    let profile = &context.profile.active_profile;
    let profile_version = &context.profile.active_profile.version.to_string();

    match &context.profile.profile_version_state {
        crate::profile::ProfileVersionState::Invalid => {
            rsx! {

                div { class: "dialog",
                    p { "Profile {profile.name} is version \"{profile_version}\" which is invalid" }
                }
            }
        }
        crate::profile::ProfileVersionState::NewerThanCurrent => {
            rsx! {

                div { class: "dialog",
                    p {
                        "Profile {profile.name} is version \"{profile_version}\" but the current version is {CURRENT_VERSION}"
                    }
                    p { "Thi profile is for a newer version of the program" }
                }
            }
        }
        crate::profile::ProfileVersionState::OlderThanCurrent => {
            if upgrade_requested.read().0 {
                rsx! {
                    ProfileUpgrade { name: profile.name.clone() }
                }
            } else {
                rsx! {

                    div { class: "dialog",
                        p {
                            "Profile {profile.name} is version \"{profile_version}\" but the current version is {CURRENT_VERSION}"
                        }
                        p { "Do you want to upgrade it to the current version?" }

                        button { onclick: move |_| upgrade_requested.set(UpgradeRequested(true)),
                            "OK"
                        }
                    }
                }
            }
        }
        crate::profile::ProfileVersionState::Current => {
            for module_id in module_registrations.0.keys() {
                let active = if context.profile.active_profile.modules.contains_key(module_id) {
                    "Active"
                } else {
                    "inactive"
                };

                modules.insert(module_id.clone(), active);
            }

            let branch = if create_info::GIT_BRANCH.is_empty() {
                "N/A".to_string()
            } else {
                let mut s = create_info::GIT_BRANCH.to_string();
                if create_info::GIT_DIRTY {
                    s.push_str("-dirty");
                }
                if create_info::GIT_STAGED {
                    s.push_str("-staged");
                }
                s
            };
            // Where the profile file and cache are, see data_home.
            // Not where it was set in screenshots, which set it, see screenshots.rs, as someone running the app would not.
            let source = crate::data_home::data_home_source().filter(|_| !crate::screenshots::running());
            let data_directory = match (crate::data_home::data_home(), source) {
                (Ok(path), Some(source)) => format!("{} (set by {})", path.display(), source),
                (Ok(path), None) => path.display().to_string(),
                (Err(error), _) => error.to_string(),
            };

            let args = &context.args.app_args;
            let debug = if args.debug {
                rsx! {
                    h2 { "Debug" }
                    h3 { "Args" }
                    table {
                        tr {
                            td { "--profile" }
                            td { {format!("{:?}", args.profile)} }
                        }
                        tr {
                            td { "--modules" }
                            td { {format!("{:?}", args.modules)} }
                        }
                        tr {
                            td { "--debug" }
                            td { {format!("{:?}", args.debug)} }
                        }
                        tr {
                            td { "--verbose" }
                            td { {format!("{:?}", args.verbose)} }
                        }
                    }
                    h3 { "Profile" }
                    table {
                        tr {
                            td { "Active Profile:" }
                            td { "{context.profile.active_profile.name}" }
                        }
                    }
                }
            } else {
                rsx!("")
            }?;

            let modules_page = rsx! {
                div {
                    // h1 { "This is Home #{xid}!" }
                    // Sized here too, so that it is not shown at the image's full size until the stylesheet has loaded.
                    img { class: "home-icon", src: APP_ICON, alt: "Bruce's Smart Home", width: "128", height: "128" }
                    h1 { "Modules" }
                    for (module_id , active) in modules {
                        div {
                            Link {
                                class: "nav-item",
                                to: Route::Module {
                                    module_id: module_id.clone(),
                                },
                                "{module_id}"
                            }
                            " [{active}]"
                        }
                    }
                    h2 { "Data Directory" }
                    p { "{data_directory}" }
                    h2 { "Build Info" }
                    table {
                        tr {
                            td { "Package Name:" }
                            td { "{create_info::PACKAGE_NAME}" }
                        }
                        tr {
                            td { "Package Version:" }
                            td { "{create_info::PACKAGE_VERSION}" }
                        }
                        tr {
                            td { "User Agent:" }
                            td { "{create_info::USER_AGENT}" }
                        }
                        tr {
                            td { "Build Timestamp (UTC):" }
                            td { "{create_info::BUILD_TIMESTAMP}" }
                        }
                        // tr {
                        //     td { "Git Repository:" }
                        //     td { "{create_info::GIT_REPOSITORY}" }
                        // }
                        tr {
                            td { "Git Branch:" }
                            td { "{branch}" }
                        }
                        tr {
                            td { "Git Tag:" }
                            td { "{create_info::GIT_TAG}" }
                        }
                                        // tr {
                    //     td { "Git Dirty:" }
                    //     td { "{create_info::GIT_DIRTY}" }
                    // }
                    // tr {
                    //     td { "Git Staged:" }
                    //     td { "{create_info::GIT_STAGED}" }
                    // }
                    // tr {
                    //     td { "Git Stash:" }
                    //     td { "{create_info::GIT_STASH}" }
                    // }
                    }
                    {debug}
                }
            };

            // Keyed by the profile, so that a page made for another, e.g. the Site page with its topology being edited,
            // is made again for this one when the profile changes while it is shown.
            let profile_key = profile.name.clone();
            rsx! {
                SidebarLayout {
                    pages: HOME_PAGES.to_vec(),
                    active: page(),
                    on_select: move |path: &'static str| page.set(path),
                    div { key: "{profile_key}", class: "home-page",
                        match page() {
                            "electricity" => rsx! { super::electricity::ElectricityPage {} },
                            "site" => rsx! { super::SitePage {} },
                            "live" => rsx! { super::LivePage {} },
                            "polling" => rsx! { PollingPage {} },
                            "devices" => rsx! { super::Devices {} },
                            "climate" => rsx! { super::ClimatePage {} },
                            _ => modules_page,
                        }
                    }
                }
            }
        }
    }
}

/// The pages of Home.
const HOME_PAGES: [PageInfo; 7] = [
    PageInfo {
        label: "Electricity",
        path: "electricity",
    },
    PageInfo {
        label: "Live",
        path: "live",
    },
    PageInfo {
        label: "Site",
        path: "site",
    },
    PageInfo {
        label: "Modules",
        path: "modules",
    },
    PageInfo {
        label: "Polling",
        path: "polling",
    },
    PageInfo {
        label: "Devices",
        path: "devices",
    },
    PageInfo {
        label: "Temp & Humidity",
        path: "climate",
    },
];

/// Upgrades the profile, see profile::upgrade_profile, which may take a minute or two, e.g. converting its data into the
/// store, off the page's thread, saying how far it is, then makes the upgraded profile the active one. Cancel stops it
/// at its next file, which, like a failure, leaves the profile and its old data as they were, so that OK returns to the
/// question of whether to upgrade it.
#[component]
fn ProfileUpgrade(name: String) -> Element {
    let mut context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let mut upgrade_requested = use_context::<Signal<UpgradeRequested>>();
    let mut error = use_signal(|| None::<String>);
    let mut cancelled = use_signal(|| false);
    // What the upgrade is doing, the latest of what it says, see convert::convert_profile.
    let mut progress = use_signal(String::new);
    let cancel = use_hook(bruce_db::progress::Cancel::new);
    use_hook(|| {
        let name = name.clone();
        let cancel = cancel.clone();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<String>();
        spawn(async move {
            while let Some(message) = receiver.recv().await {
                progress.set(message);
            }
        });
        spawn(async move {
            let upgraded = tokio::task::spawn_blocking(move || {
                let mut reporter = bruce_db::progress::Reporter::new(|message: &str| {
                    let _ = sender.send(message.to_string());
                })
                .with_cancel(cancel);
                crate::profile::upgrade_profile(&name, &mut reporter)
            })
            .await;
            match upgraded {
                Ok(Ok(profile)) => context_signal.set(Some(AppContext::for_profile(crate::Args::parse_all(), profile))),
                Ok(Err(upgrade_error)) if upgrade_error.is::<bruce_db::progress::Cancelled>() => cancelled.set(true),
                Ok(Err(upgrade_error)) => error.set(Some(format!("{upgrade_error:#}"))),
                Err(join_error) => error.set(Some(join_error.to_string())),
            }
        });
    });
    let stopped = cancelled() || error().is_some();
    rsx! {
        div { class: "dialog",
            if cancelled() {
                p { "The upgrade of profile {name} was cancelled. The profile, and its old data, are as they were, so it can be upgraded again." }
            }
            else if let Some(error) = error() {
                p { class: "error",
                    crate::components::error_text::ErrorText { error, message: format!("Unable to upgrade profile {name}") }
                }
                p { "The profile, and its old data, are as they were, so it can be upgraded again." }
            }
            else {
                p { "Upgrading profile {name} to version {CURRENT_VERSION}, which keeps its data in a new store: its data is being copied into it, which may take a minute or two. Its old data is left as it was." }
                p { "{progress}" }
            }
            if stopped {
                button { onclick: move |_| upgrade_requested.set(UpgradeRequested(false)), "OK" }
            }
            else {
                button {
                    disabled: cancel.is_cancelled(),
                    onclick: {
                        let cancel = cancel.clone();
                        move |_| cancel.cancel()
                    },
                    if cancel.is_cancelled() { "Cancelling..." } else { "Cancel" }
                }
            }
        }
    }
}
