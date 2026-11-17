use std::collections::BTreeMap;
use std::fmt::Display;
use std::time::Duration;

use bruce_graphql::GraphQLCaller;
use bruce_graphql::GraphQLQueryBuilder;
use bruce_graphql::types::{Date, DateTime};
use dioxus::prelude::*;

use crate::data_set::MonthlyDataSet;
use crate::{CacheConfig, NULL};

use super::graphql::meter;

pub enum MeterType {
    Gas,
    Electricity,
}

impl Display for MeterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeterType::Gas => write!(f, "Gas"),
            MeterType::Electricity => write!(f, "Electricity"),
        }
    }
}

// Formerly on MeterManager, kept for reference.
// pub async fn demand_handler(&self, _args: std::str::SplitWhitespace<'_>, account_number: &String) ->  anyhow::Result<()> {
//     let properties = PropertyDataSet::new(account_number, &self.cache_config, &*self.request_manager).await?;
//     for property in &properties.data_set.data.account_.properties_ {
//         for network in &property.smart_device_networks_ {
//             for device in &network.smart_devices_ {
//                 if let super::graphql::DeviceType::Esme =  device.type_ {
//                     let mut cnt=5;
//                     let ten_seconds = Duration::new(10, 0);

//                     let now = DateTime::now_utc();
//                     let now_mod_ten = now.replace_second(now.second()%10).unwrap();
//                     let mut end_timestamp = now_mod_ten.unix_timestamp() + 10;

//                     while cnt>0 {
//                         cnt -= 1;

//                         let start = DateTime::from_unix_timestamp(end_timestamp - 60)?;
//                         let end = DateTime::from_unix_timestamp(end_timestamp)?;

//                         let query = meter::get_current_demand::Query::builder()
//                             .with_meter_device_id(device.device_id_.clone())
//                             .with_start(start)
//                             .with_end(end)
//                             .with_grouping(crate::octopus::graphql::TelemetryGrouping::TenSeconds)
//                             .build()?;
//                         let demand = &self.request_manager.call(&query).await?;

//                         if demand.smart_meter_telemetry_.is_empty() {
//                             println!("NO RESULT");
//                         }
//                         else {
//                             let result = demand.smart_meter_telemetry_.get(demand.smart_meter_telemetry_.len() - 1).unwrap();
//                             println!("{} at {}", result.demand_, result.read_at_);
//                         }
//                         end_timestamp += 10;
//                         sleep(ten_seconds).await;
//                     }
//                 }
//             }
//         }
//     }

//     Ok(())
// }

#[derive(Debug)]
pub enum Tariff {
    Electricity(meter::meter_agreements::ElectricityTariffType),
    Gas(meter::meter_agreements::GasTariffType),
}

fn format_rate(rate: f64) -> String {
    format!("{:7.4}", rate)
}

fn gui_rate(rate: f64) -> Element {
    rsx! {
        td { class: "numeric", {format_rate(rate)} }
    }
}

fn gui_opt_rate(rate: Option<f64>) -> Element {
    if let Some(rate) = rate {
        gui_rate(rate)
    } else {
        rsx! {
            td { class: "numeric invalid", {NULL} }
        }
    }
}

/// The name Octopus gives a unit rate's type, e.g. EV_DEVICE_OFF_PEAK, as its Display is JSON, in quotes.
fn rate_type_name(rate_type: &super::graphql::NonBespokeElectricityRateTypeChoices) -> String {
    serde_json::to_value(rate_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn format_opt_str<T: ToString>(str: &Option<T>) -> String {
    if let Some(str) = str {
        str.to_string()
    } else {
        "UNKNOWN".to_string()
    }
}

fn gui_opt_str<T: ToString>(str: &Option<T>) -> Element {
    if let Some(str) = str {
        rsx! {
            td { {str.to_string()} }
        }
    } else {
        rsx! {
            td { class: "invalid", {NULL} }
        }
    }
}

impl Tariff {
    pub fn standing_charge(&self) -> Option<f64> {
        match self {
            Tariff::Electricity(electricity_tariff_type) => match electricity_tariff_type {
                meter::meter_agreements::ElectricityTariffType::StandardTariff(tariff) => {
                    tariff.pre_vat_standing_charge_
                }
                meter::meter_agreements::ElectricityTariffType::DayNightTariff(tariff) => {
                    tariff.pre_vat_standing_charge_
                }
                meter::meter_agreements::ElectricityTariffType::ThreeRateTariff(tariff) => {
                    tariff.pre_vat_standing_charge_
                }
                meter::meter_agreements::ElectricityTariffType::HalfHourlyTariff(tariff) => {
                    tariff.pre_vat_standing_charge_
                }
                meter::meter_agreements::ElectricityTariffType::PrepayTariff(tariff) => tariff.pre_vat_standing_charge_,
                meter::meter_agreements::ElectricityTariffType::FourRateEvTariff(tariff) => {
                    tariff.pre_vat_standing_charge_
                }
            },
            Tariff::Gas(tariff) => tariff.pre_vat_standing_charge_,
        }
    }

    pub fn gui_display(&self) -> Element {
        match self {
            Tariff::Electricity(electricity_tariff_type) => {
                match electricity_tariff_type {
                    meter::meter_agreements::ElectricityTariffType::StandardTariff(tariff) => {
                        rsx! {
                            div {
                                h3 { "Electricity Tariff: {tariff.display_name_}" }
                                table { class: "display",
                                    tr {
                                        th { class: "row-header", "Full Name" }
                                        td { {tariff.full_name_.as_str()} }
                                    }
                                    tr {
                                        th { class: "row-header", "Code" }
                                        td { {tariff.tariff_code_.as_str()} }
                                    }
                                    tr {
                                        th { class: "row-header", "Pre-VAT Standing Charge" }
                                        {gui_opt_rate(tariff.pre_vat_standing_charge_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Standing Charge" }
                                        {gui_opt_rate(tariff.standing_charge_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Pre-VAT Unit Rate" }
                                        {gui_opt_rate(tariff.pre_vat_unit_rate_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Unit Rate" }
                                        {gui_opt_rate(tariff.unit_rate_)}
                                    }
                                }
                            }
                        }
                    }
                    meter::meter_agreements::ElectricityTariffType::DayNightTariff(_tariff) => todo!(),
                    meter::meter_agreements::ElectricityTariffType::ThreeRateTariff(_tariff) => todo!(),
                    meter::meter_agreements::ElectricityTariffType::HalfHourlyTariff(tariff) => {
                        rsx! {
                            div {
                                h3 { "Electricity Tariff: {format_opt_str(&tariff.display_name_)}" }
                                table { class: "display",
                                    tr {
                                        th { class: "row-header", "Full Name" }
                                        {gui_opt_str(&tariff.full_name_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Display Name" }
                                        {gui_opt_str(&tariff.display_name_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Tariff Code" }
                                        {gui_opt_str(&tariff.tariff_code_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Product Code" }
                                        {gui_opt_str(&tariff.product_code_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Pre-VAT Standing Charge" }
                                        {gui_opt_rate(tariff.pre_vat_standing_charge_)}
                                    }
                                    tr {
                                        th { class: "row-header", "Standing Charge" }
                                        {gui_opt_rate(tariff.standing_charge_)}
                                    }

                                }
                            if !tariff.unit_rates_.is_empty() {
                                h4 { "Unit Rates" }
                                table { class: "display",
                                    tr {
                                        th { "Rate Type" }
                                        th { "Valid From" }
                                        th { "Valid To" }
                                        th { "Pre-VAT Unit Rate" }
                                        th { "Unit Rate" }
                                    }
                                    for rate in &tariff.unit_rates_ {
                                        tr {
                                            {gui_opt_str(&rate.rate_type_.as_ref().map(rate_type_name))}
                                            {gui_opt_str(&rate.valid_from_)}
                                            // No end is the usual case, a rate which is still current.
                                            td { {rate.valid_to_.as_ref().map(|valid_to| valid_to.to_string()).unwrap_or_default()} }
                                            {gui_opt_rate(rate.pre_vat_value_)}
                                            {gui_opt_rate(rate.value_)}
                                        }
                                    }
                                }
                            }
                            }
                        }
                    }
                    meter::meter_agreements::ElectricityTariffType::PrepayTariff(_tariff) => todo!(),
                    meter::meter_agreements::ElectricityTariffType::FourRateEvTariff(_tariff) => todo!(),
                }
            }
            Tariff::Gas(gas_tariff_type) => {
                rsx! {
                    div {
                        h3 { "Gas Tariff: {gas_tariff_type.full_name_}" }
                        table { class: "display",
                            tr {
                                th { class: "row-header", "Code" }
                                td { {gas_tariff_type.tariff_code_.as_str()} }
                            }
                            tr {
                                th { class: "row-header", "Standing Charge" }
                                {gui_opt_rate(gas_tariff_type.standing_charge_)}
                            }
                            tr {
                                th { class: "row-header", "Pre-VAT Unit Rate" }
                                {gui_opt_rate(gas_tariff_type.pre_vat_unit_rate_)}
                            }
                            tr {
                                th { class: "row-header", "Unit Rate" }
                                {gui_opt_rate(gas_tariff_type.unit_rate_)}
                            }
                        }
                    }
                }
            }
        }
    }
}

impl meter::electricity_agreement_line_items::AgreementInterface {
    pub fn as_electricity_agreement(self) -> meter::electricity_agreement_line_items::ElectricityAgreementType {
        match self {
            meter::electricity_agreement_line_items::AgreementInterface::ElectricityAgreementType(
                electricity_agreement_type,
            ) => electricity_agreement_type,
            meter::electricity_agreement_line_items::AgreementInterface::GasAgreementType(
                _abstract_agreement_interface,
            ) => unreachable!(),
        }
    }
    pub fn as_electricity_agreement_ref(&self) -> &meter::electricity_agreement_line_items::ElectricityAgreementType {
        match self {
            meter::electricity_agreement_line_items::AgreementInterface::ElectricityAgreementType(
                electricity_agreement_type,
            ) => electricity_agreement_type,
            meter::electricity_agreement_line_items::AgreementInterface::GasAgreementType(
                _abstract_agreement_interface,
            ) => unreachable!(),
        }
    }
}

impl meter::gas_agreement_line_items::AgreementInterface {
    pub fn as_gas_agreement(self) -> meter::gas_agreement_line_items::GasAgreementType {
        match self {
            meter::gas_agreement_line_items::AgreementInterface::ElectricityAgreementType(
                _electricity_agreement_type,
            ) => unreachable!(),
            meter::gas_agreement_line_items::AgreementInterface::GasAgreementType(abstract_agreement_interface) => {
                abstract_agreement_interface
            }
        }
    }

    pub fn as_gas_agreement_ref(&self) -> &meter::gas_agreement_line_items::GasAgreementType {
        match self {
            meter::gas_agreement_line_items::AgreementInterface::ElectricityAgreementType(
                _electricity_agreement_type,
            ) => unreachable!(),
            meter::gas_agreement_line_items::AgreementInterface::GasAgreementType(abstract_agreement_interface) => {
                abstract_agreement_interface
            }
        }
    }
}

impl Into<meter::electricity_agreement_line_items::LineItemType> for meter::gas_agreement_line_items::LineItemType {
    fn into(self) -> meter::electricity_agreement_line_items::LineItemType {
        meter::electricity_agreement_line_items::LineItemType {
            start_at_: self.start_at_,
            end_at_: self.end_at_,
            net_amount_: self.net_amount_,
            number_of_units_: self.number_of_units_,
            settlement_unit_: self.settlement_unit_,
        }
    }
}

#[allow(dead_code)]
pub struct AgreementLineItems {
    pub account_number: String,
    pub agreement_id: String,
    start_date: Date,
    pub(crate) end_date: Date,
    pub line_items: BTreeMap<
        DateTime,
        (
            crate::data_set::DataSetAttributes,
            meter::electricity_agreement_line_items::LineItemType,
        ),
    >,
}

impl AgreementLineItems {
    pub(crate) async fn new(
        config: &CacheConfig,
        request_manager: &impl GraphQLCaller,
        account_number: String,
        meter_type: &MeterType,
        agreement_id: String,
        date: &Date,
        billing_timezone: &time_tz::Tz,
    ) -> anyhow::Result<Self> {
        let hash_key = format!(
            "{}#{}#{}AgreementTransactions",
            account_number, agreement_id, meter_type
        );

        let (start_date, end_date, line_items): (
            Date,
            Date,
            std::collections::BTreeMap<
                DateTime,
                (
                    crate::data_set::DataSetAttributes,
                    meter::electricity_agreement_line_items::LineItemType,
                ),
            >,
        ) = match meter_type {
            MeterType::Gas => {
                let initial_query_provider =
                    |bucket_start: &Date, last_record: Option<&meter::gas_agreement_line_items::LineItemType>| {
                        let start_date = if let Some(last_record) = last_record {
                            last_record.start_at_.clone()
                        } else {
                            bucket_start.at_midnight(billing_timezone)
                        };
                        meter::gas_agreement_line_items::Query::builder()
                            .with_agreement_id(agreement_id.clone())
                            .with_start_at(start_date)
                            .with_timezone(String::from("Europe/London"))
                            .with_item_type(super::graphql::LineItemTypeOptions::ConsumptionCharge)
                            .with_line_item_grouping(super::graphql::LineItemGroupingOptions::None)
                            .with_first(50)
                            .build()
                    };
                let continuation_query_provider =
                    |_bucket_start: &Date, last_response: &meter::gas_agreement_line_items::Response| {
                        let agreement = last_response.gas_agreement_.as_gas_agreement_ref();
                        if let Some(end_cursor) = agreement
                            .line_items_
                            .page_info
                            .end_cursor
                            .clone()
                            .filter(|_| agreement.line_items_.page_info.has_next_page)
                        {
                            let start_date = agreement.line_items_.edges.last().unwrap().node.start_at_.clone();
                            Some(
                                meter::gas_agreement_line_items::Query::builder()
                                    .with_agreement_id(agreement_id.clone())
                                    .with_start_at(start_date)
                                    .with_timezone(String::from("Europe/London"))
                                    .with_item_type(super::graphql::LineItemTypeOptions::ConsumptionCharge)
                                    .with_line_item_grouping(super::graphql::LineItemGroupingOptions::None)
                                    .with_first(50)
                                    .with_after(end_cursor)
                                    .build(),
                            )
                        } else {
                            None
                        }
                    };

                let response_iterator = |response: meter::gas_agreement_line_items::Response| {
                    response
                        .gas_agreement_
                        .as_gas_agreement()
                        .line_items_
                        .edges
                        .into_iter()
                        .map(|edge| edge.node)
                };

                let indexer = |item: &meter::gas_agreement_line_items::LineItemType| (item.start_at_.clone(), true);

                let data_set = MonthlyDataSet::new(
                    &hash_key,
                    date,
                    Duration::from_hours(24),
                    // TODO: let the user reload a bill's line items, as for charging sessions, in case they are wrong.
                    false,
                    initial_query_provider,
                    continuation_query_provider,
                    response_iterator,
                    indexer,
                    config,
                    request_manager,
                    &bruce_graphql::NoProgress,
                )
                .await?;

                let mut data = BTreeMap::new();
                for (k, (a, v)) in data_set.data {
                    data.insert(k, (a, v.into()));
                }

                (data_set.start_date, data_set.next_date, data)
            }
            MeterType::Electricity => {
                let initial_query_provider =
                    |bucket_start: &Date,
                     last_record: Option<&meter::electricity_agreement_line_items::LineItemType>| {
                        let start_date = if let Some(last_record) = last_record {
                            last_record.start_at_.clone()
                        } else {
                            bucket_start.at_midnight(billing_timezone)
                        };

                        meter::electricity_agreement_line_items::Query::builder()
                            .with_agreement_id(agreement_id.clone())
                            .with_start_at(start_date)
                            .with_timezone(String::from("Europe/London"))
                            .with_item_type(super::graphql::LineItemTypeOptions::ConsumptionCharge)
                            .with_line_item_grouping(super::graphql::LineItemGroupingOptions::None)
                            .with_first(100)
                            .build()
                    };
                let continuation_query_provider =
                    |_bucket_start: &Date, last_response: &meter::electricity_agreement_line_items::Response| {
                        // let end_cursor = last_response.electricity_agreement_.get_line_items().last().unwrap().cursor;

                        let agreement = last_response.electricity_agreement_.as_electricity_agreement_ref();
                        if let Some(end_cursor) = agreement
                            .line_items_
                            .page_info
                            .end_cursor
                            .clone()
                            .filter(|_| agreement.line_items_.page_info.has_next_page)
                        {
                            let start_date = agreement.line_items_.edges.last().unwrap().node.start_at_.clone();
                            Some(
                                meter::electricity_agreement_line_items::Query::builder()
                                    .with_agreement_id(agreement_id.clone())
                                    .with_start_at(start_date.clone())
                                    .with_timezone(String::from("Europe/London"))
                                    .with_item_type(super::graphql::LineItemTypeOptions::ConsumptionCharge)
                                    .with_line_item_grouping(super::graphql::LineItemGroupingOptions::None)
                                    .with_first(100)
                                    .with_after(end_cursor)
                                    .build(),
                            )
                        } else {
                            None
                        }
                    };

                let response_iterator = |response: meter::electricity_agreement_line_items::Response| {
                    response
                        .electricity_agreement_
                        .as_electricity_agreement()
                        .line_items_
                        .edges
                        .into_iter()
                        .map(|edge| edge.node)
                };

                let indexer =
                    |item: &meter::electricity_agreement_line_items::LineItemType| (item.start_at_.clone(), true);

                let data_set = MonthlyDataSet::new(
                    &hash_key,
                    date,
                    Duration::from_hours(24),
                    // TODO: let the user reload a bill's line items, as for charging sessions, in case they are wrong.
                    false,
                    initial_query_provider,
                    continuation_query_provider,
                    response_iterator,
                    indexer,
                    config,
                    request_manager,
                    &bruce_graphql::NoProgress,
                )
                .await?;

                (data_set.start_date, data_set.next_date, data_set.data)
            }
        };

        Ok(Self {
            account_number,
            agreement_id,
            start_date,
            end_date,
            line_items,
        })
    }
}
