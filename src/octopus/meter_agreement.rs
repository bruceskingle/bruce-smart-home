use std::collections::HashMap;
use std::time::Duration;

use sparko_graphql::{GraphQLCaller, GraphQLQueryBuilder};
use sparko_graphql::types::DateTime;

use crate::CacheConfig;
use crate::data_set::MultiQueryDataSet;

use super::graphql::meter;
use super::meter::{MeterType, Tariff};

/// An agreement, which is the tariff for a meter over a period of time.
#[derive(Debug)]
pub struct MeterAgreement {
    pub agreement_id: String,
    pub tariff: Tariff,
}

pub struct MeterAgreementDataSet {
    pub import_electricity_map: HashMap<String, Vec<meter::meter_agreements::ElectricityAgreementType>>,
    pub export_electricity_map: HashMap<String, Vec<meter::meter_agreements::ElectricityAgreementType>>,
    pub gas_map: HashMap<String, Vec<meter::meter_agreements::GasAgreementType>>,
}

impl MeterAgreementDataSet {
    pub async fn new(config: &CacheConfig, request_manager: &impl GraphQLCaller, account_number: &str, meter_node_ids: &Vec<String>) -> anyhow::Result<Self> {
        let hash_key = format!("{}#MeterAgreements", account_number);
        let the_beginning: DateTime = DateTime::from_calendar_date(2000, time::Month::January, 1)?;
        let mut queries = Vec::new();

        for meter_node_id in meter_node_ids {
            let query = meter::meter_agreements::Query::builder()
                .with_meter_node_id(meter_node_id.clone())
                .with_valid_after(the_beginning.clone())
                .build();

            queries.push(query);
        }
        let data = MultiQueryDataSet::new(
            &hash_key,
            Duration::from_hours(24),
            queries,
            config,
            request_manager).await?;

        if !data._attributes.cached {
            if let Err(error) = config.store().and_then(|store| record_tariffs(store, account_number, &data.data)) {
                crate::log_warning!("Unable to record the tariffs' history, {error:#}");
            }
        }


        let mut export_electricity_map = HashMap::new();
        let mut import_electricity_map = HashMap::new();
        let mut gas_map = HashMap::new();

        for response in data.data {
            match response.node_ {
                meter::meter_agreements::Node::ElectricityMeterType(electricity_meter_type) => {
                    if electricity_meter_type.import_meter_.is_some() {
                        export_electricity_map.insert(electricity_meter_type.node_id_, electricity_meter_type.meter_point_.agreements_);
                    }
                    else {
                        import_electricity_map.insert(electricity_meter_type.node_id_, electricity_meter_type.meter_point_.agreements_);
                    }
                },
                meter::meter_agreements::Node::GasMeterType(gas_meter_type) => {
                    gas_map.insert(gas_meter_type.node_id_, gas_meter_type.meter_point_.agreements_);
                },
                _ => unreachable!()
            }
        }
        
        Ok(Self {
            export_electricity_map,
            import_electricity_map,
            gas_map,
        })
    }

}

/// The account's DataSet of each type of meter's tariffs, import, export or gas, by the hash of the account's number.
pub(super) fn history_key(store: &bruce_db::Store, account_number: &str, kind: &str) -> anyhow::Result<String> {
    let account = bruce_db::Id::of(&serde_json::json!({"octopus": account_number}))?;
    store.hash_key("octopus.tariff", &[bruce_db::Part::Id(account), bruce_db::Part::from(kind)])
}

/// The agreement with its unit rates sorted, so that the same rates in another order are not a change.
fn comparable(agreement: &serde_json::Value) -> serde_json::Value {
    let mut agreement = agreement.clone();
    if let Some(rates) = agreement["tariff"]["unitRates"].as_array_mut() {
        rates.sort_by_key(|rate| rate.to_string());
    }
    agreement
}

/// Records each agreement in force now, with its whole tariff, as Octopus gave it, when it differs from the last
/// recorded for the agreement, e.g. a new standing charge or new unit rates, in the profile's store, see store.rs, as
/// the kind `octopus.tariff`, a DataSet for each type of meter of the account, of {"seen", "meter", "agreement"} by when
/// it was seen and the agreement's ID, so that every change is kept. Octopus only returns a tariff as it is now, e.g.
/// the unit rates which apply now, so this is the only record of what it was before, and of when it changed.
/// Agreements which have ended are left as they were last recorded, as Octopus returns nulls for their rates.
fn record_tariffs(store: &bruce_db::Store, account_number: &str, responses: &[meter::meter_agreements::Response]) -> anyhow::Result<()> {
    let now = DateTime::from_unix_timestamp(crate::clock::now_utc().unix_timestamp())?;
    let in_force = |valid_from: &DateTime, valid_to: &Option<DateTime>| valid_from <= &now && valid_to.as_ref().is_none_or(|valid_to| valid_to > &now);
    let mut agreements = Vec::new();
    for response in responses {
        match &response.node_ {
            meter::meter_agreements::Node::ElectricityMeterType(meter) => {
                let kind = if meter.import_meter_.is_some() { "export" } else { "import" };
                for agreement in &meter.meter_point_.agreements_ {
                    if in_force(&agreement.valid_from_, &agreement.valid_to_) {
                        agreements.push((kind, serde_json::to_value(agreement)?));
                    }
                }
            },
            meter::meter_agreements::Node::GasMeterType(meter) => {
                for agreement in &meter.meter_point_.agreements_ {
                    if in_force(&agreement.valid_from_, &agreement.valid_to_) {
                        agreements.push(("gas", serde_json::to_value(agreement)?));
                    }
                }
            },
            _ => {},
        }
    }
    for (kind, agreement) in agreements {
        let Some(id) = agreement["id"].as_i64() else { continue };
        let hk = history_key(store, account_number, kind)?;
        // The last recorded of this agreement, records being in the order seen.
        let previous = store.all(&hk)?.into_iter().rev()
            .filter_map(|item| item.attributes)
            .find(|record| record["agreement"]["id"].as_i64() == Some(id));
        if previous.as_ref().is_some_and(|previous| comparable(&previous["agreement"]) == comparable(&agreement)) {
            continue;
        }
        if previous.is_some() {
            crate::log_info!("The {kind} tariff {} {} has changed", agreement["tariff"]["displayName"].as_str().unwrap_or(""),
                agreement["tariff"]["tariffCode"].as_str().unwrap_or(""));
        }
        let version = crate::store::version_now();
        let seen = version.time.to_text();
        let record = serde_json::json!({ "seen": seen, "meter": kind, "agreement": agreement });
        store.put(&bruce_db::Item::new(&hk, &format!("{seen}#{id}"), version, record))?;
    }
    Ok(())
}

impl MeterAgreementDataSet {
    pub fn get_in_scope(self, meter_type: &MeterType, is_export: bool, start_date: &DateTime, end_date: &DateTime) -> Vec<MeterAgreement> {
        let mut in_scope_agreements = Vec::new();

        match meter_type {
            MeterType::Gas => {
                for (_meter_node_id, agreement_vec) in self.gas_map {
                    for agreement in agreement_vec {
                        if &agreement.valid_from_ <= end_date {
                            if let Some(valid_to) = &agreement.valid_to_ {
                                if valid_to >= start_date {
                                    in_scope_agreements.push(MeterAgreement { agreement_id: agreement.id_.to_string(), tariff: Tariff::Gas(agreement.tariff_) });
                                }
                            }
                            else {
                                in_scope_agreements.push(MeterAgreement { agreement_id: agreement.id_.to_string(), tariff: Tariff::Gas(agreement.tariff_) });
                            }
                        }
                    }
                }
            },
            MeterType::Electricity => {
                for (_meter_node_id, agreement_vec) in if is_export {self.export_electricity_map} else {self.import_electricity_map} {
                    for agreement in agreement_vec {
                        if &agreement.valid_from_ <= end_date {
                            if let Some(valid_to) = &agreement.valid_to_ {
                                if valid_to >= start_date {
                                    in_scope_agreements.push(MeterAgreement { agreement_id: agreement.id_.to_string(), tariff: Tariff::Electricity(agreement.tariff_) });
                                }
                            }
                            else {
                                in_scope_agreements.push(MeterAgreement { agreement_id: agreement.id_.to_string(), tariff: Tariff::Electricity(agreement.tariff_) });
                            }
                        }
                    }
                }
            },
        }

        in_scope_agreements
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response for an import meter with an ended agreement, and one in force now, half-hourly, at the standing charge
    /// and with the rates given.
    fn response(standing_charge: f64, rates: serde_json::Value) -> meter::meter_agreements::Response {
        serde_json::from_value(serde_json::json!({
            "node": {
                "__typename": "ElectricityMeterType",
                "nodeId": "meter",
                "meterPoint": {
                    "mpan": "mpan",
                    "agreements": [{
                        "id": 0,
                        "validFrom": "2025-06-30T23:00:00Z",
                        "validTo": "2026-08-26T23:00:00Z",
                        "tariff": { "__typename": "HalfHourlyTariff", "standingCharge": standing_charge, "unitRates": [] },
                    }, {
                        "id": 1,
                        "validFrom": "2026-08-26T23:00:00Z",
                        "validTo": null,
                        "tariff": {
                            "__typename": "HalfHourlyTariff",
                            "displayName": "Intelligent Octopus Go",
                            "tariffCode": "E-1R-IOG",
                            "standingCharge": standing_charge,
                            "unitRates": rates,
                        },
                    }],
                },
            },
        })).unwrap()
    }

    #[test]
    fn a_tariff_is_recorded_again_only_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("marco-sparko-tariff-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let site = bruce_db::Id::of(&serde_json::json!({"site": "tariff test"})).unwrap();
        let store = bruce_db::Store::open(&format!("file://{}", dir.display()), &site).unwrap();
        let hk = history_key(&store, "A", "import").unwrap();
        let records = || store.all(&hk).unwrap();
        let off_peak = serde_json::json!({ "value": 6.6, "rateType": "EV_DEVICE_OFF_PEAK" });
        let peak = serde_json::json!({ "value": 36.294, "rateType": "EV_DEVICE_PEAK" });

        record_tariffs(&store, "A", &[response(50.0, serde_json::json!([off_peak, peak]))]).unwrap();
        assert_eq!(records().len(), 1);
        // The same rates in another order are not a change.
        record_tariffs(&store, "A", &[response(50.0, serde_json::json!([peak, off_peak]))]).unwrap();
        assert_eq!(records().len(), 1);
        record_tariffs(&store, "A", &[response(55.0, serde_json::json!([peak, off_peak]))]).unwrap();
        assert_eq!(records().len(), 2);
        let last = records().last().unwrap().attributes.clone().unwrap();
        assert_eq!(last["meter"], "import");
        assert_eq!(last["agreement"]["id"], 1, "only the agreement in force is recorded");
        assert_eq!(last["agreement"]["tariff"]["standingCharge"], 55.0);
        assert_eq!(last["agreement"]["tariff"]["unitRates"][0]["rateType"], "EV_DEVICE_PEAK", "kept in the order Octopus gave them");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
