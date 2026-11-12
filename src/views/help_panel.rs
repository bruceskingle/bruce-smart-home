//! The help panel on the right of the window, which shows the help page of the screen being shown, see help.rs. Links
//! to other help pages are followed in the panel, with a Back button to return, and others open in the browser. It can
//! be resized by dragging its left edge, and hidden, see Navbar and the bill window.

use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdArrowLeft, LdX},
};

use crate::help::{self, HelpLink};

const DEFAULT_WIDTH: f64 = 340.0;
const MIN_WIDTH: f64 = 200.0;

/// Sends the target of each link clicked in the panel, which are handled here rather than by the webview.
const LINK_LISTENER: &str = "
    const panel = document.getElementById('help-panel');
    if (panel) {
        panel.addEventListener('click', (event) => {
            const link = event.target.closest('a');
            if (link && panel.contains(link)) {
                event.preventDefault();
                event.stopPropagation();
                dioxus.send(link.getAttribute('href') || '');
            }
        }, true);
    }
";

#[component]
pub fn HelpPanel(topic: String, on_close: EventHandler<()>) -> Element {
    // The help page of the screen, and the pages reached from it by links, the one shown last.
    let mut screen_topic = use_signal(|| topic.clone());
    let mut followed = use_signal(Vec::<String>::new);
    let mut width = use_signal(|| DEFAULT_WIDTH);
    // Where a drag of the left edge started, its x and the width then, while it is dragged.
    let mut dragging = use_signal(|| None::<(f64, f64)>);

    // Back to the screen's help when the screen changes.
    use_effect(use_reactive((&topic,), move |(topic,)| {
        if *screen_topic.peek() != topic {
            screen_topic.set(topic);
            followed.write().clear();
        }
    }));

    let shown = followed.read().last().cloned().unwrap_or_else(|| screen_topic());

    use_effect(move || {
        spawn(async move {
            let mut links = document::eval(LINK_LISTENER);
            while let Ok(href) = links.recv::<String>().await {
                let from = followed
                    .peek()
                    .last()
                    .cloned()
                    .unwrap_or_else(|| screen_topic.peek().clone());
                match help::follow(&from, &href) {
                    HelpLink::Page(path) => {
                        followed.write().push(path);
                        document::eval(
                            "const content = document.getElementById('help-content'); if (content) { content.scrollTop = 0; }",
                        );
                    }
                    HelpLink::Web(url) => {
                        if let Err(error) = webbrowser::open(&url) {
                            crate::log_error!("Failed to open {url} in the browser: {error}");
                        }
                    }
                    HelpLink::None => {}
                }
            }
        });
    });

    let html = match help::page(&shown) {
        Some(page) => help::to_html(&shown, page.markdown),
        None => help::to_html(
            &shown,
            &format!("There is no help for this screen yet.\n\nIt would be in docs/help/{shown}"),
        ),
    };

    rsx! {
        div { id: "help-panel", class: "help-panel", style: "width: {width}px;",
            div {
                class: "help-resize",
                title: "Drag to resize the help",
                onmousedown: move |event: MouseEvent| dragging.set(Some((event.client_coordinates().x, width()))),
            }
            div { class: "help-header",
                if !followed.read().is_empty() {
                    button {
                        r#type: "button",
                        class: "help-button",
                        title: "Back to the previous help page",
                        onclick: move |_| { followed.write().pop(); },
                        Icon { icon: LdArrowLeft, width: 14, height: 14 }
                    }
                }
                span { class: "help-title", "Help" }
                button {
                    r#type: "button",
                    class: "help-button",
                    title: "Hide the help",
                    onclick: move |_| on_close.call(()),
                    Icon { icon: LdX, width: 14, height: 14 }
                }
            }
            div { id: "help-content", class: "help-content", dangerous_inner_html: "{html}" }
        }
        // While the left edge is dragged, the whole window follows the mouse, so that it is not lost over other elements.
        if let Some((start_x, start_width)) = dragging() {
            div {
                class: "help-drag-overlay",
                onmousemove: move |event: MouseEvent| {
                    let new_width = start_width + start_x - event.client_coordinates().x;
                    width.set(new_width.max(MIN_WIDTH));
                },
                onmouseup: move |_| dragging.set(None),
                onmouseleave: move |_| dragging.set(None),
            }
        }
    }
}
