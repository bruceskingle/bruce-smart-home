use bruce_graphql::types::Date;
use dioxus::prelude::*;
use dioxus_free_icons::{Icon, icons::ld_icons::LdExternalLink};
use indexmap::IndexMap;
use std::sync::Arc;
use std::time::Duration;

use crate::CacheConfig;
use crate::data_set::SingleRecordDataSet;
use bruce_graphql::{GraphQLCaller, GraphQLQueryBuilder};

use super::graphql::meter;

/*
        #[derive(Serialize, Deserialize, Debug, DisplayAsJsonPretty)]
        pub struct PropertyType {
            #[serde(rename = "id")]
            pub id_: String, // T1
            #[serde(rename = "address")]
            pub address_: String, // T1
            #[serde(rename = "postcode")]
            pub postcode_: String, // T1
            #[serde(rename = "occupancyPeriods")]
            pub occupancy_periods_: Vec<OccupancyPeriodType>, // T1
            #[serde(rename = "coordinates")]
            pub coordinates_: CoordinatesType, // T1
            #[serde(rename = "electricityMeterPoints")]
            pub electricity_meter_points_: Vec<ElectricityMeterPointType>, // T1
            #[serde(rename = "gasMeterPoints")]
            pub gas_meter_points_: Vec<GasMeterPointType>, // T1
            #[serde(rename = "smartDeviceNetworks")]
            pub smart_device_networks_: Vec<SmartMeterDeviceNetworkType>, // T1
        }
*/

fn gui_opt_date(date: &Option<Date>) -> String {
    if let Some(date) = date {
        date.to_string()
    } else {
        "".to_string()
    }
}

pub type Property = meter::account_properties_meters::PropertyType;

impl Property {
    pub fn gui_summary_header() -> Element {
        rsx! {
            tr { class: "header",
                th { "ID" }
                th { "Address" }
                th { "Post Code" }
                th { "Coordinates" }
                th { "Occupancy Periods" }
            }
        }
    }

    fn format_period(period: &meter::account_properties_meters::OccupancyPeriodType) -> String {
        format!(
            "{}-{}",
            period.effective_from_,
            if let Some(effective_to) = &period.effective_to_ {
                effective_to.to_string()
            } else {
                "".to_string()
            }
        )
    }

    pub fn gui_summary_line(&self) -> Element {
        // let row_class = if attributes.cached { "cached"} else {"fetched"};
        let id = self.id_.clone();
        let mut it = self.occupancy_periods_.iter();
        let period = if let Some(period_data) = it.next() {
            Self::format_period(period_data)
        } else {
            String::from("No occupancy periods")
        };

        let mut additional_periods = Vec::new();
        for period_data in it {
            let period_str = Self::format_period(period_data);
            additional_periods.push(rsx! {
                tr {
                    td { colspan: 4, "" }
                    td { "{period_str}" }
                }
            });
        }

        let coordinates = format!("{:.3} {:.3}", self.coordinates_.latitude_, self.coordinates_.longitude_);
        let map_url = format!(
            "https://www.google.com/maps/search/?api=1&query={},{}",
            self.coordinates_.latitude_, self.coordinates_.longitude_
        );

        rsx! {
            tr {
                td {
                    div {
                        class: "link",
                        // onclick: |_| {path_signal.set(vec!(String::from("bills"), abstract_bill.id_.clone()))},
                        onclick: move |_| {
                            // println!("Id={}", id);
                            // let id = abstract_bill.id_.clone();
                            // nav_callback(id);
                            let mut path_signal = use_context::<Signal<Vec<String>>>();
                            let new_path = vec![String::from("properties"), id.clone()];
                            path_signal.set(new_path);
                        },
                        "{self.id_}"
                    }
                }
                td { "{self.address_}" }
                td { "{self.postcode_}" }
                td {
                    a { href: "{map_url}", target: "_blank",
                        "{coordinates}"
                        Icon {
                            class: "inline-icon",
                            icon: LdExternalLink,
                            width: 12,
                            height: 12,
                            title: "Opens in a new window",
                        }
                    }
                }

                td { "{period}" }
            }
            for item in additional_periods {
                {item}
            }
        }
    }

    pub fn gui_display(&self) -> Element {
        let mut it = self.occupancy_periods_.iter();
        let period = if let Some(period_data) = it.next() {
            Self::format_period(period_data)
        } else {
            String::from("No occupancy periods")
        };

        let mut additional_periods = Vec::new();
        for period_data in it {
            let period_str = Self::format_period(period_data);
            additional_periods.push(rsx! {
                tr {
                    td { "" }
                    td { "{period_str}" }
                }
            });
        }

        // A row for each meter, with its meter point's details on the first row of the meter point's meters, or a row
        // with just the meter point if it has no meters.
        let mut meter_rows = Vec::new();
        for meter_point in &self.electricity_meter_points_ {
            // Meter points link to their measurements, consumption for import and generation for export.
            let mpan = meter_point.mpan_.clone();
            let mut point_cells = Some(rsx! {
                td { "Electricity" }
                td { "{meter_point.id_}" }
                td {
                    span {
                        class: "link",
                        onclick: move |_| {
                            let mut path_signal = use_context::<Signal<Vec<String>>>();
                            path_signal.set(vec![String::from("measurements"), mpan.clone()]);
                        },
                        "{meter_point.mpan_}"
                    }
                }
                td { "{meter_point.status_}" }
                td { "{gui_opt_date(&meter_point.supply_end_date_)}" }
            });
            if meter_point.meters_.is_empty() {
                meter_rows.push(rsx! { tr { {point_cells.take()} td { colspan: 5 } } });
            }
            for (index, meter) in meter_point.meters_.iter().enumerate() {
                let direction = if meter.import_meter_.is_none() {
                    "Import"
                } else {
                    "Export"
                };
                // Only the first row of the meter point has its details.
                let point_cells = if index == 0 {
                    point_cells.take()
                } else {
                    Some(rsx! { td { colspan: 5 } })
                };
                meter_rows.push(rsx! {
                    tr {
                        {point_cells}
                        td { "{meter.node_id_}" }
                        td { "{meter.serial_number_}" }
                        td { "{meter.consumption_units_}" }
                        td { "{meter.has_and_allows_hh_readings_}" }
                        td { "{direction}" }
                    }
                });
            }
        }
        for meter_point in &self.gas_meter_points_ {
            let mut point_cells = Some(rsx! {
                td { "Gas" }
                td { "{meter_point.id_}" }
                td { "{meter_point.mprn_}" }
                td { "{meter_point.status_}" }
                td { "{gui_opt_date(&meter_point.supply_end_date_)}" }
            });
            if meter_point.meters_.is_empty() {
                meter_rows.push(rsx! { tr { {point_cells.take()} td { colspan: 5 } } });
            }
            for (index, meter) in meter_point.meters_.iter().enumerate() {
                // Only the first row of the meter point has its details.
                let point_cells = if index == 0 {
                    point_cells.take()
                } else {
                    Some(rsx! { td { colspan: 5 } })
                };
                meter_rows.push(rsx! {
                    tr {
                        {point_cells}
                        td { "{meter.node_id_}" }
                        td { "{meter.serial_number_}" }
                        td { "{meter.consumption_units_}" }
                        td { "{meter.has_and_allows_hh_readings_}" }
                        td {}
                    }
                });
            }
        }

        let coordinates = format!("{:.3} {:.3}", self.coordinates_.latitude_, self.coordinates_.longitude_);
        let map_url = format!(
            "https://www.google.com/maps/search/?api=1&query={},{}",
            self.coordinates_.latitude_, self.coordinates_.longitude_
        );

        rsx! {
            h1 { "Property Details" }
            table { class: "display",
                tr {
                    th { class: "row-header", "ID" }
                    td { "{self.id_}" }
                }
                tr {
                    th { class: "row-header", "Address" }
                    td { "{self.address_}" }
                }
                tr {
                    th { class: "row-header", "Post Code" }
                    td { "{self.postcode_}" }
                }
                tr {
                    th { class: "row-header", "Coordinates" }
                    td {
                        a { href: "{map_url}", target: "_blank",
                            "{coordinates}"
                            Icon {
                                class: "inline-icon",
                                icon: LdExternalLink,
                                width: 12,
                                height: 12,
                                title: "Opens in a new window",
                            }
                        }
                    }
                }
                tr {
                    th { class: "row-header", "Occupancy Periods" }
                    td { "{period}" }
                }
                for item in additional_periods {
                    {item}
                }
            }
            h2 { "Meters" }
            table { class: "display",
                tr { class: "header",
                    th { "Fuel" }
                    th { "Meter Point ID" }
                    th { "MPAN / MPRN" }
                    th { "Status" }
                    th { "Supply End" }
                    th { "Meter Node ID" }
                    th { "Serial Number" }
                    th { "Units" }
                    th { title: "Has and allows half hourly readings", "Half Hourly" }
                    th { "Direction" }
                }
                for row in meter_rows {
                    {row}
                }
            }
            h2 { "Smart Device Network" }
            {smart_device_network_table(&[self])}
        }
    }
}

pub struct PropertyDataSet {
    pub property_map: IndexMap<String, Arc<meter::account_properties_meters::PropertyType>>,
    pub meter_node_ids: Vec<String>,
}

impl PropertyDataSet {
    pub async fn new(
        account_number: &String,
        config: &Arc<CacheConfig>,
        request_manager: &impl GraphQLCaller,
    ) -> anyhow::Result<Self> {
        let hash_key = format!("{}#Properties", account_number);
        let query_provider = || {
            meter::account_properties_meters::Query::builder()
                .with_account_number(account_number.clone())
                .build()
        };
        let data_set = SingleRecordDataSet::new(
            &hash_key,
            Duration::from_hours(20 * 24), // 30 days
            query_provider,
            config,
            request_manager,
        )
        .await?;

        let mut meter_node_ids: Vec<String> = Vec::new();
        let mut property_map = IndexMap::new();

        for property in data_set.data.account_.properties_ {
            for point in &property.electricity_meter_points_ {
                for meter in &point.meters_ {
                    meter_node_ids.push(meter.node_id_.clone());
                }
            }
            for point in &property.gas_meter_points_ {
                for meter in &point.meters_ {
                    meter_node_ids.push(meter.node_id_.clone());
                }
            }
            property_map.insert(property.id_.clone(), Arc::new(property));
        }

        Ok(Self {
            property_map,
            meter_node_ids,
        })
    }
}

/// A table of the devices on the smart meter networks of the properties, with a column for the property if there is
/// more than one, and for the network if a property has more than one. The ID of a consumer access device, such as a
/// Home Mini, links to the readings it provides of the electricity meter (ESME) on its network.
pub fn smart_device_network_table(properties: &[&meter::account_properties_meters::PropertyType]) -> Element {
    let several_properties = properties.len() > 1;
    let several_networks = properties
        .iter()
        .any(|property| property.smart_device_networks_.len() > 1);
    let device_type = |device: &meter::account_properties_meters::SmartMeterDeviceType| {
        device.type_.to_string().trim_matches('"').to_string()
    };
    let devices: Vec<(String, usize, String, String, &'static str, Option<String>)> = properties
        .iter()
        .flat_map(|property| {
            property
                .smart_device_networks_
                .iter()
                .enumerate()
                .flat_map(move |(network, devices)| {
                    let esme = devices
                        .smart_devices_
                        .iter()
                        .find(|device| device_type(device) == "ESME")
                        .map(|device| device.device_id_.clone());
                    devices.smart_devices_.iter().map(move |device| {
                        let device_type = device_type(device);
                        let description = device_type_description(&device_type);
                        // Only a consumer access device links to the readings.
                        let readings_of = if device_type == "CAD" { esme.clone() } else { None };
                        (
                            property.address_.clone(),
                            network + 1,
                            device.device_id_.clone(),
                            device_type,
                            description,
                            readings_of,
                        )
                    })
                })
        })
        .collect();

    if devices.is_empty() {
        return rsx! { p { "No smart meter network" } };
    }
    rsx! {
        table { class: "display",
            tr { class: "header",
                if several_properties {
                    th { "Property" }
                }
                if several_networks {
                    th { "Network" }
                }
                th { "Device ID" }
                th { "Type" }
                th { "Description" }
            }
            for (address, network, device_id, device_type, description, readings_of) in devices {
                tr {
                    if several_properties {
                        td { "{address}" }
                    }
                    if several_networks {
                        td { "{network}" }
                    }
                    td {
                        if let Some(esme_id) = readings_of {
                            span {
                                class: "link",
                                title: "Show the meter's readings every 10 seconds",
                                onclick: {
                                    let device_id = device_id.clone();
                                    move |_| {
                                        let mut path_signal = use_context::<Signal<Vec<String>>>();
                                        path_signal.set(vec![
                                            String::from("measurements"),
                                            String::from("telemetry"),
                                            esme_id.clone(),
                                            device_id.clone(),
                                        ]);
                                    }
                                },
                                "{device_id}"
                            }
                        }
                        else {
                            "{device_id}"
                        }
                    }
                    td { "{device_type}" }
                    td { "{description}" }
                }
            }
        }
    }
}

/// What a type of device on a smart meter network is, from the SMETS names.
fn device_type_description(device_type: &str) -> &'static str {
    match device_type {
        "ESME" => "Electricity smart meter",
        "GSME" => "Gas smart meter",
        "GPF" => "Gas proxy function, the gas meter's data on the communications hub",
        "CHF" => "Communications hub",
        "HCALCS" => "Load control switch",
        "PPMID" => "In-home display (prepayment)",
        "IHD" => "In-home display",
        "CAD" => "Consumer access device, e.g. an Octopus Home Mini",
        "IHD_OR_CAD" => "In-home display or consumer access device",
        _ => "",
    }
}
