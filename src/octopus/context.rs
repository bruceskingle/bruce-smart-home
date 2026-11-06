use std::sync::Arc;

use dioxus::prelude::*;

use crate::CacheConfig;

use super::RequestManager;

/// The state shared by all the pages of the Octopus module, provided to them as a Dioxus context by OctopusRoot.
/// Pages get it with use_context::<Arc<OctopusContext>>().
pub struct OctopusContext {
    pub account_id: String,
    pub billing_timezone: &'static time_tz::Tz,
    pub cache_config: Arc<CacheConfig>,
    pub request_manager: Arc<RequestManager>,
    /// The profile's store, see store.rs, which the module's data is moving into from the cache.
    pub store: Arc<bruce_db::Store>,
}

#[derive(Props, Clone)]
pub struct OctopusRootProps {
    context: Arc<OctopusContext>,
    children: Element,
}

impl PartialEq for OctopusRootProps {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.context, &other.context)
            && self.children == other.children
    }
}

/// Provides the OctopusContext to the page rendered inside it.
///
/// A context is only provided when the component is created, so this must be given a key which changes when the
/// context does, otherwise pages would keep using the old one.
#[component]
pub fn OctopusRoot(props: OctopusRootProps) -> Element {
    use_context_provider(|| props.context.clone());
    props.children
}
