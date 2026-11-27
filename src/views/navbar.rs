use std::sync::Arc;

use crate::components::error_text::ErrorText;
use crate::views::help_panel::HelpPanel;
use crate::views::log_panel::LogPanel;
use crate::{
    AppContext, UpgradeRequested,
    components::{
        app::Route, copy_domain::CopyDomainDialog, delete_domain::DeleteDomainDialog, rename_domain::RenameDomainDialog,
    },
    domain::{Settings, domain_names_equal},
};
use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdCircleHelp, LdScrollText, LdSettings, LdUser},
};

// use crate::DOMAIN_MANAGER;

const APP_ICON: Asset = asset!("/assets/app-icon.png");

/// The Navbar component that will be rendered on all pages of our app since every page is under the layout.
///
///
/// This layout component wraps the UI of [Route::Home] and [Route::Blog] in a common navbar. The contents of the Home and Blog
/// routes will be rendered under the outlet inside this component
#[component]
/// A navigation bar with left-aligned items and a right-aligned dropdown.
/// Designed for Dioxus Desktop (but works for web too).
pub fn Navbar() -> Element {
    let mut context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let mut upgrade_requested =
        use_context_provider::<Signal<UpgradeRequested>>(|| Signal::new(UpgradeRequested(false)));
    let context = context_signal.read().as_ref().unwrap().clone();

    // State for the dropdown menu
    let mut menu_open = use_signal(|| false);
    let mut new_domain_open = use_signal(|| false);
    let mut new_domain_generation = use_signal(|| 0u32);
    let mut new_domain_name = use_signal(String::new);
    let mut new_domain_error = use_signal(|| None::<String>);
    // The domain whose ... menu is open, and the domains being deleted, renamed and copied.
    let mut domain_actions_open = use_signal(|| None::<String>);
    let mut delete_domain = use_signal(|| None::<String>);
    let mut rename_domain = use_signal(|| None::<String>);
    let mut copy_domain = use_signal(|| None::<String>);

    // The active domain's settings, for the Settings page and the log panel, which follow the domain when it changes.
    let mut settings = use_context_provider(|| Signal::new(context.domain.active_domain.settings.clone()));
    use_effect(move || {
        if let Some(context) = context_signal.read().as_ref() {
            settings.set(context.domain.active_domain.settings.clone());
        }
    });
    // Not in screenshots, see screenshots.rs.
    let show_log = settings.read().log.show && !crate::screenshots::running();
    // Shows or hides the log panel, and saves it so that it stays that way.
    let set_show_log = {
        let domain_name = context.domain.active_domain.name.clone();
        move |show: bool| {
            let mut new_settings: Settings = settings.read().clone();
            new_settings.log.show = show;
            if let Err(error) = crate::domain::update_settings(&domain_name, &new_settings) {
                crate::log_error!("Failed to save whether the log is shown: {error}");
            }
            settings.set(new_settings);
        }
    };

    // Whether the help is shown, in every window, see help::use_help_shown.
    let mut help_shown = crate::help::use_help_shown();
    let show_help = help_shown();
    let mut set_show_help = move |show: bool| {
        crate::help::set_shown(show);
        help_shown.set(show);
    };
    // The help of the module screen being shown, which the Module view sets.
    let module_help_topic = use_context_provider(|| crate::help::ModuleHelpTopic(Signal::new(None)));
    // The help of the Home page being shown, which Home sets.
    let home_help_topic = use_context_provider(|| crate::help::HomeHelpTopic(Signal::new("home.md")));

    let mut all = Vec::new();
    for name in &context.domain.all_domains {
        all.push(name.clone());
    }

    // get the current route so we can mark the active nav item
    let current_route = use_route::<Route>();
    let navigator = use_navigator();
    let help_topic = match &current_route {
        Route::Home {} => String::from(home_help_topic.0()),
        Route::DomainSize { .. } => String::from("domain-size.md"),
        Route::Settings {} => String::from("settings.md"),
        Route::Module { module_id } => module_help_topic.0().unwrap_or_else(|| format!("{module_id}/index.md")),
    };

    rsx! {
        div { class: "app-layout",
        nav { class: "nav",
            // The app's icon, filling the height of the bar, which also goes to the home page.
            Link {
                class: "nav-logo",
                to: Route::Home {},
                title: "Bruce's Smart Home",
                style: "background-image: url({APP_ICON});",
            }
            div { class: "nav-left",

                Link {
                    class: if matches!(&current_route, Route::Home {} | Route::DomainSize { .. }) { "nav-item active" } else { "nav-item" },
                    to: Route::Home {},
                    "Home"
                }

                // By name, as on Home, as the domain's modules are in a HashMap, in no order.
                for module_id in context.domain.active_domain.modules.keys().cloned().collect::<std::collections::BTreeSet<_>>() {
                    Link {
                        class: match &current_route {
                            Route::Module { module_id: mid } if *mid == module_id => "nav-item active",
                            _ => "nav-item",
                        },
                        to: Route::Module {
                            module_id: module_id.clone(),
                        },
                        "{module_id}"
                    }
                }
            }

            div { class: "spacer" }

            div { class: "nav-right",
                button {
                    r#type: "button",
                    class: if show_help { "icon-button active" } else { "icon-button" },
                    title: if show_help { "Hide the help" } else { "Show the help" },
                    onclick: move |_| set_show_help(!show_help),
                    Icon { icon: LdCircleHelp, width: 16, height: 16 }
                }
                button {
                    r#type: "button",
                    class: if show_log { "icon-button active" } else { "icon-button" },
                    title: if show_log { "Hide the log" } else { "Show the log" },
                    onclick: {
                        let mut set_show_log = set_show_log.clone();
                        move |_| set_show_log(!show_log)
                    },
                    Icon { icon: LdScrollText, width: 16, height: 16 }
                }
                div { class: "dropdown",
                    button {
                        key: "{new_domain_generation}",
                        class: "dropdown-toggle",
                        onclick: move |_| {
                            if !*new_domain_open.read() {
                                let is_open = *menu_open.read();
                                menu_open.set(!is_open);
                            }
                        },
                        // ARIA roles for accessibility
                        // aria_has_popup: "true",
                        aria_expanded: "{menu_open}",

                        if *new_domain_open.read() {
                            if let Some(error) = &*new_domain_error.read() {
                                span { class: "invalid", "{error}" }
                            }
                            input {
                                class: "domain-rename",
                                r#type: "text",
                                placeholder: "Domain name",
                                autofocus: true,
                                onmounted: move |event| async move {
                                    let _ = event.data().set_focus(true).await;
                                },
                                value: "{new_domain_name}",
                                oninput: move |event| new_domain_name.set(event.value().clone()),
                                onclick: move |event| event.stop_propagation(),
                                onkeydown: move |event| {
                                    match event.key().to_string().as_str() {
                                        "Enter" => {
                                            let domain_name = new_domain_name.read().clone();
                                            match crate::domain::create_domain(&domain_name) {
                                                Ok(active_domain) => {
                                                    // Its domain is made first, on the Devices page, before it has a
                                                    // store, which is then opened under the domain's ID.
                                                    let name = active_domain.active_domain.name.clone();
                                                    if let Err(error) = crate::domain::await_founding(&name) {
                                                        crate::log_warning!("Unable to note that the domain {name} awaits its founding, {error:#}");
                                                    }
                                                    crate::domain_security::note_new_domain(&name);

                                                    // start
                                                    // let new_context = Arc::new(AppContext {
                                                    //     args: crate::Args::parse_all(),
                                                    //     domain: active_domain,
                                                    // });
                                                    // context_signal.set(Some(new_context));
                                                    // end
                                                    let new_context = AppContext::for_domain(crate::Args::parse_all(), active_domain);
                                                    context_signal.set(Some(new_context));
                                                    //next
                                                    upgrade_requested.set(UpgradeRequested(false));
                                                    new_domain_open.set(false);
                                                    new_domain_name.set(String::new());
                                                    new_domain_error.set(None);
                                                    navigator.replace(Route::Home {});
                                                }
                                                Err(error) => new_domain_error.set(Some(error.to_string())),
                                            }
                                        }
                                        "Escape" => {
                                            new_domain_open.set(false);
                                            new_domain_name.set(String::new());
                                            new_domain_error.set(None);
                                        }
                                        _ => {}
                                    }
                                },
                            }
                        } else {
                            Icon { class: "inline-icon", icon: LdUser, width: 14, height: 14 }
                            " Domain {context.domain.active_domain.name} "
                            svg {
                                width: "14",
                                height: "14",
                                view_box: "0 0 24 24",
                                path {
                                    d: "M6 9l6 6 6-6",
                                    stroke: "currentColor",
                                    stroke_width: "2",
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                }
                            }
                        }
                    }

                    // If menu is open, render overlay
                    if *menu_open.read() {
                        div {
                            class: "overlay",
                            onclick: move |_| {
                                menu_open.set(false);
                                domain_actions_open.set(None);
                            },
                        }
                    }

                    // Dropdown menu
                    div { class: if *menu_open.read() { "menu open" } else { "menu" },

                        for name in all {
                            div {
                                class: "menu-item domain-item",
                                onclick: {
                                    let name = name.clone();
                                    move |_| {
                                        menu_open.set(false);
                                        domain_actions_open.set(None);
                                        let new_context = AppContext::for_domain(crate::Args::parse_all(), crate::domain::set_active_domain(&name)?);

                                        context_signal.set(Some(new_context));
                                        upgrade_requested.set(UpgradeRequested(false));
                                        navigator.replace(Route::Home {});
                                        Ok(())
                                    }
                                },
                                span { "{&name}" }
                                // The active domain can only be copied, as the app has its files open, and its size seen.
                                span { class: "domain-actions",
                                    button {
                                        class: "domain-actions-toggle",
                                        title: "Domain actions",
                                        onclick: {
                                            let name = name.clone();
                                            move |event: MouseEvent| {
                                                event.stop_propagation();
                                                let open = domain_actions_open.read().as_ref() == Some(&name);
                                                domain_actions_open.set(if open { None } else { Some(name.clone()) });
                                            }
                                        },
                                        "…"
                                    }
                                    if domain_actions_open.read().as_ref() == Some(&name) {
                                        div { class: "domain-actions-menu",
                                            div {
                                                class: "menu-item",
                                                onclick: {
                                                    let name = name.clone();
                                                    move |event: MouseEvent| {
                                                        event.stop_propagation();
                                                        menu_open.set(false);
                                                        domain_actions_open.set(None);
                                                        navigator.push(Route::DomainSize { name: name.clone() });
                                                    }
                                                },
                                                "Size..."
                                            }
                                            div {
                                                class: "menu-item",
                                                onclick: {
                                                    let name = name.clone();
                                                    move |event: MouseEvent| {
                                                        event.stop_propagation();
                                                        menu_open.set(false);
                                                        domain_actions_open.set(None);
                                                        copy_domain.set(Some(name.clone()));
                                                    }
                                                },
                                                "Copy..."
                                            }
                                            if !domain_names_equal(&name, &context.domain.active_domain.name) {
                                                div {
                                                    class: "menu-item",
                                                    onclick: {
                                                        let name = name.clone();
                                                        move |event: MouseEvent| {
                                                            event.stop_propagation();
                                                            menu_open.set(false);
                                                            domain_actions_open.set(None);
                                                            rename_domain.set(Some(name.clone()));
                                                        }
                                                    },
                                                    "Rename..."
                                                }
                                                div {
                                                    class: "menu-item",
                                                    onclick: {
                                                        let name = name.clone();
                                                        move |event: MouseEvent| {
                                                            event.stop_propagation();
                                                            menu_open.set(false);
                                                            domain_actions_open.set(None);
                                                            delete_domain.set(Some(name.clone()));
                                                        }
                                                    },
                                                    "Delete..."
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        div {
                            class: "menu-item create-new",
                            onclick: move |_| {
                                menu_open.set(false);
                                new_domain_open.set(true);
                                let generation = *new_domain_generation.read() + 1;
                                new_domain_generation.set(generation);
                                new_domain_error.set(None);
                                new_domain_name.set(String::new());
                            },
                            "Create New Domain"
                        }
                    }
                }
                button {
                    r#type: "button",
                    class: if matches!(current_route, Route::Settings {}) { "icon-button active" } else { "icon-button" },
                    title: "Settings",
                    onclick: move |_| { navigator.push(Route::Settings {}); },
                    Icon { icon: LdSettings, width: 16, height: 16 }
                }
            }
        }

        if let Some(name) = delete_domain.read().clone() {
            DeleteDomainDialog { name, on_close: move |_| delete_domain.set(None) }
        }
        if let Some(name) = rename_domain.read().clone() {
            RenameDomainDialog { name, on_close: move |_| rename_domain.set(None) }
        }
        if let Some(name) = copy_domain.read().clone() {
            CopyDomainDialog { name, on_close: move |_| copy_domain.set(None) }
        }

        div { class: "app-body",
            div { class: "app-page",
                ErrorBoundary {
                    handle_error: |errors: ErrorContext| rsx! { PageError { errors } },
                    Outlet::<Route> {}
                }
            }
            if show_help {
                HelpPanel {
                    topic: help_topic,
                    on_close: move |_| set_show_help(false),
                }
            }
        }
        if show_log {
            LogPanel {
                on_close: {
                    let mut set_show_log = set_show_log.clone();
                    move |_| set_show_log(false)
                },
            }
        }
        }
    }
}

/// What the page area shows instead of a page which fails, e.g. as its data could not be loaded, so that the rest of the
/// app, e.g. the navigation bar, can still be used, until another page is chosen, which clears it.
#[component]
fn PageError(errors: ErrorContext) -> Element {
    let route = use_route::<Route>();
    let failed_on = use_hook(|| route.clone());
    let error = errors.error().map(|error| format!("{error:#}")).unwrap_or_default();
    use_hook(|| crate::log_error!("{error}"));
    use_effect(use_reactive!(|route| if route != failed_on {
        errors.clear_errors();
    }));
    rsx! {
        p {
            "This page could not be shown: "
            ErrorText { error }
        }
    }
}
