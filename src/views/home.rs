use std::{collections::BTreeMap, sync::Arc};

use crate::{MarcoSparkoContext, ModuleRegistrations, PageInfo, UpgradeRequested, components::app::Route, profile::CURRENT_VERSION};

use super::polling::PollingPage;
use super::sidebar::SidebarLayout;
use dioxus::prelude::*;


include!(concat!(env!("OUT_DIR"), "/crate_info.rs"));

const APP_ICON: Asset = asset!("/assets/MarcoSparko.png");

/// The Home page component that will be rendered when the current route is `[Route::Home]`
#[component]
pub fn Home(

    // xid: i32,
    // modules_signal: ModuleRegistrations
) -> Element {
    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
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
        },
        crate::profile::ProfileVersionState::NewerThanCurrent => {
            rsx! {

                div { class: "dialog",
                    p {
                        "Profile {profile.name} is version \"{profile_version}\" but the current version is {CURRENT_VERSION}"
                    }
                    p { "Thi profile is for a newer version of the program" }
                }
            }
        },
        crate::profile::ProfileVersionState::OlderThanCurrent => {
            if upgrade_requested.read().0 {
                rsx! {
                    p { "Upgrade requested" }
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
        },
        crate::profile::ProfileVersionState::Current => {
            for module_id in module_registrations.0.keys() {
                let active = if context.profile.active_profile.modules.contains_key(module_id) {
                    "Active"
                }
                else {
                    "inactive"
                };

                modules.insert(module_id.clone(),active);
            }

            
            let branch = if create_info::GIT_BRANCH.is_empty() {
                "N/A".to_string()
            } else {
                let mut s = create_info::GIT_BRANCH.to_string();
                if create_info::GIT_DIRTY  {
                    
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

            let args = &context.args.marco_sparko_args;
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
            }
            else {
                rsx!( "" )
            }?;

            let modules_page = rsx! {
                div {
                    // h1 { "This is Home #{xid}!" }
                    // Sized here too, so that it is not shown at the image's full size until the stylesheet has loaded.
                    img { class: "home-icon", src: APP_ICON, alt: "Marco Sparko", width: "128", height: "128" }
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

            rsx! {
                SidebarLayout {
                    pages: HOME_PAGES.to_vec(),
                    active: page(),
                    on_select: move |path: &'static str| page.set(path),
                    match page() {
                        "electricity" => rsx! { super::electricity::ElectricityPage {} },
                        "site" => rsx! { super::SitePage {} },
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

/// The pages of Home.
const HOME_PAGES: [PageInfo; 6] = [
    PageInfo { label: "Electricity", path: "electricity" },
    PageInfo { label: "Site", path: "site" },
    PageInfo { label: "Modules", path: "modules" },
    PageInfo { label: "Polling", path: "polling" },
    PageInfo { label: "Devices", path: "devices" },
    PageInfo { label: "Temp & Humidity", path: "climate" },
];
