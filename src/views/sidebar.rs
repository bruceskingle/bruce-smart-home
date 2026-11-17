use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdPanelLeftClose, LdPanelLeftOpen},
};

use crate::PageInfo;

/// A page with the list of pages beside it, e.g. a module's or Home's, which can be hidden. active is the path of the
/// page shown, and on_select is given the path of a page chosen from the list.
#[component]
pub fn SidebarLayout(
    pages: Vec<PageInfo>,
    active: String,
    on_select: EventHandler<&'static str>,
    children: Element,
) -> Element {
    let mut sidebar_open = use_signal(|| true);
    // Hidden while a screenshot is taken, unless the shot is about it, see screenshots.rs.
    let sidebar_shown = *sidebar_open.read() && !crate::screenshots::PAGE_LIST_HIDDEN();

    rsx! {
        div { class: "container",
            // Sidebar, which when hidden is a narrow strip with just the button to show it again.
            nav {
                class: format_args!("sidebar {}", if sidebar_shown { "open" } else { "closed" }),
                aria_label: "secondary navigation",

                button {
                    class: "sidebar-toggle",
                    r#type: "button",
                    title: if sidebar_shown { "Hide the page list" } else { "Show the page list" },
                    onclick: move |_| sidebar_open.set(!sidebar_shown),
                    if sidebar_shown {
                        Icon { icon: LdPanelLeftClose, width: 18, height: 18 }
                    } else {
                        Icon { icon: LdPanelLeftOpen, width: 18, height: 18 }
                    }
                }
                div { class: "sidebar-inner",
                    for page_info in pages {
                        div {
                            class: format_args!("sidebar-item {}", if active == page_info.path { "active" } else { "inactive" }),
                            onclick: move |_| on_select.call(page_info.path),
                            "{page_info.label}"
                        }
                    }
                }
            }

            // Main content
            main { class: "content", {children} }
        }
    }
}
