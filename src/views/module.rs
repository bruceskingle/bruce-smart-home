use std::sync::Arc;

use dioxus::prelude::*;

use crate::{MarcoSparkoContext, ModuleFactory, ModuleRegistrations, PageInfo};

use super::sidebar::SidebarLayout;

// Simple HTML escaper
fn escape_html(input: &str) -> String {
    input
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&#x27;")
}

/// The id of the page path is for, the first page if it does not say, and the rest of the path.
fn page_for(path: &[String], page_list: &[PageInfo]) -> Option<(&'static str, Vec<String>)> {
    let page_id = path.first().map(String::as_str).unwrap_or_default();
    page_list
        .iter()
        .find(|page_info| page_id.is_empty() || page_id == page_info.path)
        .map(|page_info| (page_info.path, path.iter().skip(1).cloned().collect()))
}

fn get_page(
    module: &Box<dyn crate::Module + Send>,
    path: &Vec<String>,
    page_list: &Vec<PageInfo>,
) -> (&'static str, Element) {
    let mut page_id = "";
    let mut it = path.into_iter();

    if let Some(p) = it.next() {
        page_id = p;
    }

    let mut remaining_path = Vec::new();

    loop {
        if let Some(s) = it.next() {
            remaining_path.push((*s).clone());
        } else {
            break;
        }
    }
    if page_list.len() < 1 {
        let module_id = module.module_id();

        return ("", rsx!("Module page list is empty {module_id}"));
    }

    for page_info in page_list {
        if page_id == "" || page_id == page_info.path {
            return (page_info.path, module.get_component(page_info.path, remaining_path));
        }
    }

    let msg = format!("{:?}", path);
    ("", rsx!("Unknown page in path {msg}"))
}

/// The page of the module, re-created when it is another module's, as the router keeps the component when only the
/// module_id of the route changes, and ModulePage only builds its module once. It is a list of one, as Dioxus only
/// uses keys in lists.
#[component]
pub fn Module(module_id: String) -> Element {
    rsx! {
        for module_id in [module_id] {
            ModulePage { key: "{module_id}", module_id }
        }
    }
}

#[component]
fn ModulePage(module_id: String) -> Element {
    let mut path_signal = use_signal(|| vec![String::from("")]);
    let path = (&*path_signal.read()).clone();

    use_context_provider::<Signal<Vec<String>>>(move || path_signal);

    let context_signal = use_context::<Signal<Option<Arc<MarcoSparkoContext>>>>();
    let opt_context = &*context_signal.read();
    let context = opt_context.as_ref().unwrap();
    let module_registrations = use_context::<ModuleRegistrations>();

    let mut call_construct_module_signal = use_signal::<bool>(|| true);
    let mut construct_module_action: Action<
        (ModuleRegistrations, Arc<MarcoSparkoContext>, String),
        Arc<dyn crate::ModuleFactory + Send>,
    > = use_action(
        move |module_registrations: ModuleRegistrations,
              marco_sparko_context: std::sync::Arc<crate::MarcoSparkoContext>,
              module_id: String| async move {
            module_registrations
                .module_factory(&module_id, &marco_sparko_context)
                .await
        },
    );

    let mut call_check_ready_module_signal = use_signal::<bool>(|| true);
    let mut check_ready_module_action: Action<(Arc<dyn ModuleFactory>, bool), bool> =
        use_action(move |builder: Arc<dyn ModuleFactory>, _dummy: bool| async move { builder.is_ready().await });

    let mut call_build_module_signal = use_signal::<bool>(|| true);
    let mut build_module_action: Action<(Arc<dyn ModuleFactory>, bool), Box<dyn crate::Module + Send>> =
        use_action(move |builder: Arc<dyn ModuleFactory>, _dummy: bool| async move { builder.build().await });

    // The help of what is being shown, the login page until the module is ready.
    let help_topic = match (check_ready_module_action.value(), build_module_action.value()) {
        (_, Some(Ok(module))) => {
            let module = module.read();
            page_for(&path, &module.get_page_list()).map(|(page_id, rest)| module.help_topic(page_id, &rest))
        }
        (Some(Ok(ready)), _) if !*ready.read() => Some(format!("{module_id}/login.md")),
        _ => None,
    };
    crate::help::use_help_topic(help_topic);

    if *call_construct_module_signal.read() {
        call_construct_module_signal.set(false);
        construct_module_action.call(module_registrations.clone(), context.clone(), module_id.clone());
    }

    if let Some(result) = construct_module_action.value() {
        let builder_signal = result?;
        let builder = (*builder_signal.read()).clone();

        if *call_check_ready_module_signal.read() {
            call_check_ready_module_signal.set(false);
            check_ready_module_action.call(builder.clone(), true);
        }

        if let Some(result) = check_ready_module_action.value() {
            let is_ready_signal = result?;
            let is_ready = *is_ready_signal.read();

            if is_ready {
                if *call_build_module_signal.read() {
                    call_build_module_signal.set(false);
                    // let t =
                    build_module_action.call(builder.clone(), true);
                }

                if let Some(result) = build_module_action.value() {
                    let module_signal = result?;
                    let module = &*module_signal.read();
                    let page_list = module.get_page_list();
                    let (active_page_id, content) = get_page(&module, &path, &page_list);

                    let body = match content {
                        Ok(element) => rsx!(
                            div { class: "filler", {element} }
                        )?,
                        Err(error) => {
                            let escaped_error = escape_html(&format!("{:?}", &error));

                            let html = format!(
                                "<div class=\"error\">Failed to load page content: <pre>{}</pre></div>",
                                escaped_error
                            );
                            rsx! {
                                div { dangerous_inner_html: "{html}" }
                            }?
                        }
                    };

                    rsx! {
                        SidebarLayout {
                            pages: page_list,
                            active: active_page_id,
                            on_select: move |page: &'static str| path_signal.set(vec![String::from(page)]),
                            {body}
                        }
                    }
                } else {
                    rsx! { "Building {module_id}..." }
                }
            } else {
                builder.init_page()
            }
        } else {
            rsx! { "Loading {module_id}..." }
        }
    } else {
        rsx! { "Loading {module_id}..." }
    }
}
