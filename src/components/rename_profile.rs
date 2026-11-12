use std::sync::Arc;

use dioxus::prelude::*;

use crate::AppContext;

/// A modal dialog asking for a profile's new name. OK renames it, see profile::rename_profile, and reloads the
/// profiles so that the menu lists it by its new name, Cancel calls on_close.
#[component]
pub fn RenameProfileDialog(name: String, on_close: EventHandler<()>) -> Element {
    let mut context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let mut new_name = use_signal(|| name.clone());
    let mut error = use_signal(|| None::<String>);

    let mut rename = {
        let name = name.clone();
        move || {
            let context = context_signal.read().as_ref().unwrap().clone();
            let active_name = context.profile.active_profile.name.clone();
            let result = crate::profile::rename_profile(&name, &new_name.read(), &active_name)
                .and_then(|_| crate::profile::fetch_active_profile(&Some(active_name)));
            match result {
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
        }
    };

    rsx! {
        div { class: "modal-backdrop",
            div { class: "modal profile-dialog",
                h3 { "Rename {name}" }
                input {
                    r#type: "text",
                    value: "{new_name}",
                    onmounted: move |event| async move {
                        let _ = event.data().set_focus(true).await;
                    },
                    oninput: move |event| new_name.set(event.value()),
                    onkeydown: {
                        let mut rename = rename.clone();
                        move |event: KeyboardEvent| match event.key().to_string().as_str() {
                            "Enter" => rename(),
                            "Escape" => on_close.call(()),
                            _ => {},
                        }
                    },
                }
                p { "Its settings, logins, downloaded data and log keep the new name." }
                if let Some(error) = &*error.read() {
                    div { class: "modal-error", "{error}" }
                }
                div { class: "modal-buttons",
                    button { onclick: move |_| rename(), "OK" }
                    button { onclick: move |_| on_close.call(()), "Cancel" }
                }
            }
        }
    }
}
