//! A password or API key field with a button which shows what it holds, the same on every module's login page.

use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdEye, LdEyeOff},
};

/// An input for a secret, hidden as dots until its Show button is pressed, so that a saved password or key, which the
/// login page fills in, can be checked, e.g. after the source refused it, see login.rs. It starts hidden every time
/// the page is shown.
#[component]
pub fn SecretInput(name: String, value: Signal<String>) -> Element {
    let mut value = value;
    let mut shown = use_signal(|| false);
    rsx! {
        span { class: "secret-input",
            input {
                r#type: if shown() { "text" } else { "password" },
                name,
                autocomplete: "off",
                spellcheck: "false",
                value: "{value}",
                oninput: move |e| value.set(e.value()),
            }
            button {
                r#type: "button",
                class: "icon-button",
                aria_pressed: shown(),
                // Only an icon, so its name is given to screen readers; a clickable thing has no tooltip, see AGENTS.md.
                aria_label: if shown() { "Hide" } else { "Show" },
                onclick: move |_| shown.toggle(),
                if shown() {
                    Icon { icon: LdEyeOff, width: 16, height: 16 }
                } else {
                    Icon { icon: LdEye, width: 16, height: 16 }
                }
            }
        }
    }
}
