//! The EV Charging page: the account's charge points and vehicles, and the charging sessions of each.

use std::sync::Arc;

use anyhow::anyhow;
use dioxus::prelude::*;

use super::charging_sessions::ChargingSessions;
use crate::octopus::context::OctopusContext;
use crate::octopus::smart_device::{SmartDevice, SmartDeviceDataSet};

#[derive(Props, Clone, PartialEq)]
pub struct EvChargingPageProps {
    path: Vec<String>,
}

#[component]
pub fn EvChargingPage(props: EvChargingPageProps) -> Element {
    let context = use_context::<Arc<OctopusContext>>();

    let devices_resource = use_resource({
        let context = context.clone();
        move || {
            let context = context.clone();
            async move {
                SmartDeviceDataSet::new(&context.account_id, &context.cache_config, &*context.request_manager).await
            }
        }
    });

    match &*devices_resource.read() {
        None => rsx! {
            p { "Loading devices..." }
        },
        Some(Err(error)) => Err(anyhow!("Failed to load devices: {error:#}"))?,
        Some(Ok(device_list)) => {
            let chargers: Vec<SmartDevice> = device_list
                .devices
                .iter()
                .filter(|device| device.charging.is_some())
                .cloned()
                .collect();

            // Are we looking at one charger? Its path is its ID.
            if let Some(device_id) = props.path.first() {
                let device = chargers.iter().find(|device| &device.id == device_id);
                let device_name = device
                    .and_then(|device| device.name.clone())
                    .unwrap_or_else(|| device_id.clone());
                let battery_size = device.and_then(|device| device.battery_size);
                rsx! {
                    ChargingSessions {
                        key: "{device_id}",
                        account_id: context.account_id.clone(),
                        device_id: device_id.clone(),
                        device_name,
                        battery_size,
                        billing_timezone: context.billing_timezone,
                    }
                }
            } else if chargers.is_empty() {
                rsx! {
                    h1 { "EV Smart Charge Devices" }
                    p { "You don't seem to have an EV charger." }
                }
            } else {
                rsx! {
                    h1 { "EV Smart Charge Devices" }
                    table {
                        tr { class: "header",
                            th { "Name" }
                            th { "Kind" }
                            th { "Provider" }
                            th { title: "The make of the EV configured in Octopus",
                                "Make"
                            }
                            th { title: "The model of the EV configured in Octopus",
                                "Model"
                            }
                            th { title: "The battery size Octopus has for the configured EV",
                                "Configured Battery Size"
                            }
                            th { "Charger Power" }
                            th {}
                        }
                        for device in chargers {
                            tr {
                                td { {device.name.clone().unwrap_or_default()} }
                                td { {device.charging.unwrap_or_default()} }
                                td { "{device.provider}" }
                                td { {device.make.clone().unwrap_or_default()} }
                                td { {device.model.clone().unwrap_or_default()} }
                                td {
                                    {device.battery_size.map(|size| format!("{size} kWh")).unwrap_or_default()}
                                }
                                td {
                                    {device.power.map(|power| format!("{power} kW")).unwrap_or_default()}
                                }
                                td {
                                    span {
                                        class: "link",
                                        onclick: {
                                            let id = device.id.clone();
                                            move |_| {
                                                let mut path_signal = use_context::<Signal<Vec<String>>>();
                                                path_signal.set(vec![String::from("ev"), id.clone()]);
                                            }
                                        },
                                        "Charging Sessions"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
