//! A button of a view toggle group, which can be unavailable with a reason shown when it is clicked.

use dioxus::prelude::*;

/// A button in a .view-toggle group. When unavailable is set the button looks disabled but, unlike a disabled button,
/// is still clicked, and then shows unavailable in a popup under the click until the mouse leaves it, as well as in its
/// tooltip. The popup follows the button rather than being in it, so that it is not faded with it.
#[component]
pub fn ToggleButton(
    label: String,
    active: bool,
    /// Disabled with no explanation.
    #[props(default)]
    disabled: bool,
    #[props(default)] unavailable: Option<String>,
    onclick: EventHandler<()>,
) -> Element {
    // Where the button was clicked to show the popup, in client coordinates.
    let mut showing = use_signal(|| None::<(f64, f64)>);
    let unavailable_click = unavailable.is_some();

    rsx! {
        button {
            r#type: "button",
            class: format_args!("{}{}", if active { "active" } else { "" }, if unavailable_click { " unavailable" } else { "" }),
            disabled,
            aria_disabled: unavailable_click,
            title: unavailable.clone(),
            onclick: move |event: MouseEvent| {
                if unavailable_click {
                    let point = event.client_coordinates();
                    showing.set(if showing().is_some() { None } else { Some((point.x, point.y)) });
                }
                else {
                    onclick.call(());
                }
            },
            onmouseleave: move |_| showing.set(None),
            "{label}"
        }
        if let (Some((x, y)), Some(unavailable)) = (showing(), &unavailable) {
            span { class: "toggle-hint", style: "left: {x}px; top: {y}px;", "{unavailable}" }
        }
    }
}
