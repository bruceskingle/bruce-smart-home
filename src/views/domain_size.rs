use std::sync::Arc;

use dioxus::prelude::*;

use crate::{
    AppContext,
    components::{app::Route, delete_domain::DeleteDomainDialog},
    domain::{domain_names_equal, format_size},
};

/// The disk space used by a domain, by plugin, with buttons to delete the domain or return to the home page.
#[component]
pub fn DomainSize(name: String) -> Element {
    let context_signal = use_context::<Signal<Option<Arc<AppContext>>>>();
    let context = context_signal.read().as_ref().unwrap().clone();
    let navigator = use_navigator();
    let mut delete_open = use_signal(|| false);
    let is_active = domain_names_equal(&name, &context.domain.active_domain.name);

    // Walking the cache directory may take a while, so it is done off the UI thread.
    let usage = use_resource({
        let name = name.clone();
        move || {
            let name = name.clone();
            async move {
                tokio::task::spawn_blocking(move || crate::domain::domain_cache_usage(&name))
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|result| result.map_err(|error| error.to_string()))
            }
        }
    });

    rsx! {
        div { class: "domain-size",
            h1 { "Domain {name}" }
            match &*usage.read() {
                None => rsx! { p { "Calculating..." } },
                Some(Err(error)) => rsx! { p { class: "invalid", "{error}" } },
                Some(Ok(usage)) => rsx! {
                    table {
                        tr {
                            th { "Plugin" }
                            th { "Disk Space" }
                        }
                        for (plugin, size) in usage.plugins.iter() {
                            tr {
                                td { "{plugin}" }
                                td { class: "size", {format_size(*size)} }
                            }
                        }
                        tr { class: "total",
                            td { "Total" }
                            td { class: "size", {format_size(usage.total())} }
                        }
                    }
                },
            }
            div { class: "domain-size-buttons",
                button {
                    class: "danger",
                    disabled: is_active,
                    title: if is_active { "The active domain cannot be deleted" },
                    onclick: move |_| delete_open.set(true),
                    "Delete"
                }
                button { onclick: move |_| { navigator.push(Route::Home {}); }, "OK" }
            }
        }
        if delete_open() {
            DeleteDomainDialog { name: name.clone(), on_close: move |_| delete_open.set(false) }
        }
    }
}
