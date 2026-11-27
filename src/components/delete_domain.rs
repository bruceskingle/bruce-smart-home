use std::sync::Arc;

use dioxus::prelude::*;

use crate::{AppContext, components::app::Route};

/// A modal dialog asking whether to delete the given domain. OK deletes its tokens, moves its cached data to the trash,
/// removes it from the domain file and goes to the home page, Cancel calls on_close.
#[component]
pub fn DeleteDomainDialog(name: String, on_close: EventHandler<()>) -> Element {
    let mut context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let navigator = use_navigator();
    let mut error = use_signal(|| None::<String>);

    let delete = {
        let name = name.clone();
        move |_| {
            let context = context_signal.read().as_ref().unwrap().clone();
            let active_name = context.domain.active_domain.name.clone();
            let result = crate::domain::delete_domain(&name, &active_name)
                .and_then(|_| crate::domain::fetch_active_domain(&Some(active_name)));
            match result {
                Ok(domain) => {
                    // Reload the domain so that the deleted one is no longer listed.
                    context_signal.set(Some(Arc::new(AppContext {
                        args: crate::Args::parse_all(),
                        domain,
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
                p { "Do you really want to delete the domain {name}? This cannot be undone." }
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
