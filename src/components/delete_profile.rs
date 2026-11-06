use std::sync::Arc;

use dioxus::prelude::*;

use crate::{MarcoSparkoContext, components::app::Route};

/// A modal dialog asking whether to delete the given profile. OK deletes its tokens, moves its cached data to the trash,
/// removes it from the profile file and goes to the home page, Cancel calls on_close.
#[component]
pub fn DeleteProfileDialog(name: String, on_close: EventHandler<()>) -> Element {
    let mut context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let navigator = use_navigator();
    let mut error = use_signal(|| None::<String>);

    let delete = {
        let name = name.clone();
        move |_| {
            let context = context_signal.read().as_ref().unwrap().clone();
            let active_name = context.profile.active_profile.name.clone();
            let result = crate::profile::delete_profile(&name, &active_name)
                .and_then(|_| crate::profile::fetch_active_profile(&Some(active_name)));
            match result {
                Ok(profile) => {
                    // Reload the profile so that the deleted one is no longer listed.
                    context_signal.set(Some(Arc::new(MarcoSparkoContext {
                        args: crate::Args::ms_parse(),
                        profile,
                        store: Default::default(),
                    })));
                    on_close.call(());
                    navigator.replace(Route::Home {});
                }
                Err(e) => error.set(Some(e.to_string())),
            }
        }
    };

    rsx! {
        div { class: "modal-backdrop",
            div { class: "modal danger",
                p { "Do you really want to delete the profile {name}? This cannot be undone." }
                p {
                    "Its login tokens are deleted immediately. Its cached data is moved to the trash, so the disk space is not freed until the trash is emptied."
                }
                if let Some(error) = &*error.read() {
                    div { class: "modal-error", "{error}" }
                }
                div { class: "modal-buttons",
                    button { onclick: delete, "OK" }
                    button { onclick: move |_| on_close.call(()), "Cancel" }
                }
            }
        }
    }
}
