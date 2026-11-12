use std::sync::Arc;

use dioxus::prelude::*;

use crate::AppContext;

/// A modal dialog asking for the name of a copy of a profile, and whether it is offline. OK copies it, see
/// profile::copy_profile, off the page's thread, as the downloaded data can be large, and reloads the profiles so that
/// the menu lists the copy, Cancel calls on_close.
#[component]
pub fn CopyProfileDialog(name: String, on_close: EventHandler<()>) -> Element {
    let mut context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let mut new_name = use_signal(|| format!("{name}_copy"));
    let mut offline = use_signal(|| true);
    let mut copying = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let copy = {
        let name = name.clone();
        move || {
            let (name, to, offline) = (name.clone(), new_name.read().clone(), offline());
            copying.set(true);
            error.set(None);
            spawn(async move {
                let result = tokio::task::spawn_blocking(move || crate::profile::copy_profile(&name, &to, offline))
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result);
                let active_name = context_signal
                    .read()
                    .as_ref()
                    .unwrap()
                    .profile
                    .active_profile
                    .name
                    .clone();
                match result.and_then(|_| crate::profile::fetch_active_profile(&Some(active_name))) {
                    Ok(profile) => {
                        context_signal.set(Some(Arc::new(AppContext {
                            args: crate::Args::parse_all(),
                            profile,
                            store: Default::default(),
                        })));
                        on_close.call(());
                    }
                    Err(e) => error.set(Some(format!("{e:#}"))),
                }
                copying.set(false);
            });
        }
    };

    rsx! {
        div { class: "modal-backdrop",
            div { class: "modal profile-dialog",
                h3 { "Copy {name}" }
                input {
                    r#type: "text",
                    value: "{new_name}",
                    disabled: copying(),
                    onmounted: move |event| async move {
                        let _ = event.data().set_focus(true).await;
                    },
                    oninput: move |event| new_name.set(event.value()),
                    onkeydown: {
                        let mut copy = copy.clone();
                        move |event: KeyboardEvent| match event.key().to_string().as_str() {
                            "Enter" => copy(),
                            "Escape" => on_close.call(()),
                            _ => {},
                        }
                    },
                }
                label {
                    input { r#type: "checkbox", checked: offline(), disabled: copying(), onchange: move |event| offline.set(event.checked()) }
                    " Offline"
                }
                // Both, one over the other, the other hidden, so that the dialog is as tall whichever is shown.
                div { class: "stacked-notes",
                    p { class: if !offline() { "hidden" },
                        "The copy shows the data as it is now, without logging in or fetching anything, so it can be changed, e.g. its "
                        "topology, without changing {name}. It does not have {name}'s logins or capture devices."
                    }
                    p { class: if offline() { "hidden" },
                        "The copy has {name}'s logins and capture devices, and fetches and polls as {name} does when it is the active profile."
                    }
                }
                if copying() {
                    p { "Copying..." }
                }
                if let Some(error) = &*error.read() {
                    div { class: "modal-error", "{error}" }
                }
                div { class: "modal-buttons",
                    button { disabled: copying(), onclick: { let mut copy = copy.clone(); move |_| copy() }, "OK" }
                    button { disabled: copying(), onclick: move |_| on_close.call(()), "Cancel" }
                }
            }
        }
    }
}
