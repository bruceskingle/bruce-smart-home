use std::sync::Arc;
use std::time::Duration;

use crate::CacheConfig;
use crate::data_set::SingleRecordDataSet;
use bruce_graphql::{GraphQLCaller, GraphQLQueryBuilder};

use super::decimal::Decimal;
use super::graphql::meter;

/// A smart device on the account, e.g. an EV charger, whose readings can be shown like a meter point's.
#[derive(Clone, PartialEq, Debug)]
pub struct SmartDevice {
    /// Identifies the device registration, used to filter measurements to the device.
    pub id: String,
    pub name: Option<String>,
    /// e.g. ELECTRIC_VEHICLES
    pub device_type: String,
    /// e.g. MYENERGI_V2
    pub provider: String,
    /// What kind of charging device it is, "Charge point" or "Vehicle", or None if it does not charge. Only these have
    /// charging sessions.
    pub charging: Option<&'static str>,
    /// For a charging device, the EV's make, model and battery size in kWh, and the charge point's power in kW, as far
    /// as Octopus has been told them.
    pub make: Option<String>,
    pub model: Option<String>,
    pub battery_size: Option<Decimal>,
    pub power: Option<Decimal>,
}

/// The smart devices on an account.
pub struct SmartDeviceDataSet {
    pub devices: Vec<SmartDevice>,
}

impl SmartDeviceDataSet {
    pub async fn new(account_number: &String, config: &Arc<CacheConfig>, request_manager: &impl GraphQLCaller) -> anyhow::Result<Self> {
        // Version 2 has the details of charging devices, which earlier cached responses do not.
        let hash_key = format!("{}#SmartDevices2", account_number);
        let query_provider = || {
            meter::smart_devices::Query::builder()
                .with_account_number(account_number.clone())
                .build()
        };
        let data_set = SingleRecordDataSet::new(
            &hash_key,
            Duration::from_hours(24),
            query_provider,
            config,
            request_manager
        ).await?;

        // The enums display as JSON strings, so without their quotes.
        let enum_name = |json: String| json.trim_matches('"').to_string();
        let devices = data_set.data.devices_.iter()
            .map(|device| {
                let (charging, make, model, battery_size, power) = match device {
                    meter::smart_devices::SmartFlexDeviceInterface::SmartFlexChargePoint(charge_point) => (
                        Some("Charge point"),
                        charge_point.make_.clone(),
                        charge_point.model_.clone(),
                        charge_point.vehicle_battery_size_,
                        charge_point.charge_point_power_output_,
                    ),
                    meter::smart_devices::SmartFlexDeviceInterface::SmartFlexVehicle(vehicle) => (
                        Some("Vehicle"),
                        vehicle.make_.clone(),
                        vehicle.model_.clone(),
                        vehicle.vehicle_battery_size_,
                        vehicle.charge_point_power_output_,
                    ),
                    _ => (None, None, None, None, None),
                };
                let device = device.as_smart_flex_device_interface();
                SmartDevice {
                    id: device.id_.clone(),
                    name: device.name_.clone(),
                    device_type: enum_name(device.device_type_.to_string()),
                    provider: enum_name(device.provider_.to_string()),
                    charging,
                    make,
                    model,
                    battery_size,
                    power,
                }
            })
            .collect();

        Ok(Self { devices })
    }
}
