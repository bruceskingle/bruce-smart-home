use std::sync::Arc;

use anyhow::anyhow;
use dioxus::prelude::*;

use crate::octopus::context::OctopusContext;
use crate::octopus::graphql;
use crate::octopus::property::{Property, PropertyDataSet};

#[derive(Props, Clone, PartialEq)]
pub struct PropertiesPageProps {
    path: Vec<String>,
}

#[component]
pub fn PropertiesPage(props: PropertiesPageProps) -> Element {
    let context = use_context::<Arc<OctopusContext>>();

    // Fetch the list of all properties, this runs once when the component is mounted.
    let properties_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move { PropertyDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await }
        }
    });

    match &*properties_resource.read() {
        None => rsx! {
            div { "Loading Properties for account {context.account_id}..." }
        },
        Some(Err(error)) => Err(anyhow!("Failed to load properties: {error:#}"))?,
        Some(Ok(property_list)) => {
            // Are we looking at one bill?
            if let Some(property_id) = props.path.first() {
                if let Some(property) = property_list.property_map.get(property_id) {
                    rsx! {
                        PropertyDetail {
                            key: "{property_id}",
                            property: property.clone(),
                        }
                    }
                } else {
                    rsx! {
                        div { "No such property {property_id}" }
                    }
                }
            } else {
                rsx! {
                    table {
                        {Property::gui_summary_header()?}
                        for property in property_list.property_map.values() {
                            {property.gui_summary_line()?}
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone)]
struct PropertyDetailProps {
    property: Arc<graphql::meter::account_properties_meters::PropertyType>,
}

impl PartialEq for PropertyDetailProps {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.property, &other.property)
    }
}

#[component]
fn PropertyDetail(props: PropertyDetailProps) -> Element {
    rsx! {
        {props.property.gui_display()?}
    }
}
