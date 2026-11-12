//! The samples captured, put in the store in the same form as bruce-smart-home's own Tesla snapshots
//! (src/tesla/snapshot.rs), as the app would put them, see db.rs.
//!
//! Only the fields the app uses are kept, as a whole snapshot is about 6 KB of JSON: in the store as JSON, see
//! Snapshots, and the latest in RAM as typed values, for the screen.

use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::db::Db;

/// The gateway's endpoints which are polled, after /api/, see bruce-smart-home's src/tesla/snapshot.rs LIVE. status and
/// system_status are left out, as the app does not use them in samples and system_status is the largest response.
pub const ENDPOINTS: &[&str] = &[
    "meters/aggregates",
    "system_status/soe",
    "system_status/grid_status",
    "operation",
];

/// One meter of meters/aggregates, e.g. battery, with power in W and energy in Wh, as the gateway gives them.
#[derive(Deserialize)]
pub struct Meter {
    #[serde(skip)]
    pub name: Box<str>,
    #[serde(default)]
    pub instant_power: f32,
    #[serde(default)]
    pub energy_imported: f64,
    #[serde(default)]
    pub energy_exported: f64,
    /// When the gateway last heard from the meter, to see how old its numbers are.
    #[serde(default, deserialize_with = "time_or_none")]
    pub last_communication_time: Option<DateTime<Utc>>,
}

fn time_or_none<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error> {
    let text = Option::<String>::deserialize(deserializer)?;
    Ok(text
        .and_then(|text| DateTime::parse_from_rfc3339(&text).ok())
        .map(|time| time.with_timezone(&Utc)))
}

/// What the gateway said to one of the ENDPOINTS, reduced to the fields the app uses.
pub enum Response {
    Aggregates(Vec<Meter>),
    Soe {
        percentage: Option<f64>,
    },
    GridStatus {
        grid_status: Option<Box<str>>,
        grid_services_active: Option<bool>,
    },
    Operation {
        backup_reserve_percent: Option<f64>,
        real_mode: Option<Box<str>>,
    },
}

impl Response {
    pub fn parse(endpoint: &str, body: &[u8]) -> anyhow::Result<Response> {
        #[derive(Deserialize)]
        struct Soe {
            percentage: Option<f64>,
        }
        #[derive(Deserialize)]
        struct GridStatus {
            grid_status: Option<Box<str>>,
            grid_services_active: Option<bool>,
        }
        #[derive(Deserialize)]
        struct Operation {
            backup_reserve_percent: Option<f64>,
            real_mode: Option<Box<str>>,
        }

        Ok(match endpoint {
            "meters/aggregates" => {
                // Only the entries which are meters, as the app does, see snapshot::meters.
                let aggregates: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(body)?;
                let meters = aggregates
                    .into_iter()
                    .filter(|(_, meter)| meter.get("instant_power").is_some())
                    .map(|(name, meter)| {
                        let mut meter: Meter = serde_json::from_value(meter)?;
                        meter.name = name.into();
                        Ok(meter)
                    })
                    .collect::<serde_json::Result<Vec<Meter>>>()?;
                Response::Aggregates(meters)
            }
            "system_status/soe" => {
                let soe: Soe = serde_json::from_slice(body)?;
                Response::Soe {
                    percentage: soe.percentage,
                }
            }
            "system_status/grid_status" => {
                let status: GridStatus = serde_json::from_slice(body)?;
                Response::GridStatus {
                    grid_status: status.grid_status,
                    grid_services_active: status.grid_services_active,
                }
            }
            "operation" => {
                let operation: Operation = serde_json::from_slice(body)?;
                Response::Operation {
                    backup_reserve_percent: operation.backup_reserve_percent,
                    real_mode: operation.real_mode,
                }
            }
            _ => anyhow::bail!("{endpoint} is not one of the endpoints which are polled"),
        })
    }

    /// The response as the gateway gave it, less the fields which were not kept.
    fn to_json(&self) -> serde_json::Value {
        match self {
            Response::Aggregates(meters) => {
                let meters: serde_json::Map<String, serde_json::Value> = meters
                    .iter()
                    .map(|meter| {
                        (
                            meter.name.to_string(),
                            json!({
                                "instant_power": meter.instant_power,
                                "energy_imported": meter.energy_imported,
                                "energy_exported": meter.energy_exported,
                                "last_communication_time": meter.last_communication_time.map(format_time),
                            }),
                        )
                    })
                    .collect();
                serde_json::Value::Object(meters)
            }
            Response::Soe { percentage } => json!({ "percentage": percentage }),
            Response::GridStatus {
                grid_status,
                grid_services_active,
            } => json!({
                "grid_status": grid_status,
                "grid_services_active": grid_services_active,
            }),
            Response::Operation {
                backup_reserve_percent,
                real_mode,
            } => json!({
                "backup_reserve_percent": backup_reserve_percent,
                "real_mode": real_mode,
            }),
        }
    }
}

/// One poll of the gateway: slot is the time it was scheduled for, if it was, requested when the first request was
/// sent, and received when the last response arrived, as in the app's snapshots. responses is in the order of
/// ENDPOINTS, with the error for any which failed.
pub struct Record {
    pub slot: Option<DateTime<Utc>>,
    pub requested: DateTime<Utc>,
    pub received: DateTime<Utc>,
    pub responses: Vec<Result<Response, Box<str>>>,
}

impl Record {
    /// When the record is of, as bruce-smart-home's snapshot::time_of.
    pub fn time(&self) -> DateTime<Utc> {
        self.slot.unwrap_or(self.requested)
    }

    pub fn response(&self, endpoint: &str) -> Option<&Response> {
        let index = ENDPOINTS.iter().position(|name| *name == endpoint)?;
        self.responses.get(index)?.as_ref().ok()
    }

    pub fn meter(&self, name: &str) -> Option<&Meter> {
        match self.response("meters/aggregates")? {
            Response::Aggregates(meters) => meters.iter().find(|meter| &*meter.name == name),
            _ => None,
        }
    }

    /// The record as the app's snapshots are, with captured_by, to say which device it came from, see
    /// device::captured_by, and the firmware.
    pub fn to_json(&self, captured_by: &str) -> serde_json::Value {
        let responses: serde_json::Map<String, serde_json::Value> = ENDPOINTS
            .iter()
            .zip(&self.responses)
            .map(|(endpoint, response)| {
                let json = match response {
                    Ok(response) => response.to_json(),
                    Err(error) => json!({ "error": error }),
                };
                (endpoint.to_string(), json)
            })
            .collect();
        json!({
            "slot": self.slot.map(format_time),
            "requested": format_time(self.requested),
            "received": format_time(self.received),
            "capturedBy": captured_by,
            "firmware": crate::device::FIRMWARE,
            "responses": responses,
        })
    }
}

pub fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// The latest poll, for the screen and the app: when, the error if it failed, and when a poll last succeeded, so that
/// the app can show how long a feature has been failing, see Device::to_json.
pub struct LastPoll {
    pub time: DateTime<Utc>,
    pub error: Option<String>,
    pub last_success: Option<DateTime<Utc>>,
}

impl LastPoll {
    /// A poll at time which succeeded.
    pub fn succeeded(time: DateTime<Utc>) -> LastPoll {
        LastPoll {
            time,
            error: None,
            last_success: Some(time),
        }
    }

    /// A poll at time which failed, after the previous poll, if there was one.
    pub fn failed(previous: Option<&LastPoll>, time: DateTime<Utc>, error: String) -> LastPoll {
        LastPoll {
            time,
            error: Some(error),
            last_success: previous.and_then(|previous| previous.last_success),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "time": format_time(self.time),
            "error": self.error,
            "lastSuccess": self.last_success.map(format_time),
        })
    }
}

/// The latest record, for the screen, with how the latest poll went, the records themselves being put in the store, see
/// db.rs, as the app would put them, see bruce-smart-home's src/device_kinds.rs.
pub struct Snapshots {
    latest: Option<Record>,
    last_poll: Option<LastPoll>,
    db: Arc<Db>,
    /// What took the records, in each one's capturedBy.
    captured_by: String,
}

impl Snapshots {
    pub fn new(db: Arc<Db>, captured_by: String) -> Snapshots {
        Snapshots {
            latest: None,
            last_poll: None,
            db,
            captured_by,
        }
    }

    pub fn add(&mut self, record: Record) {
        let json = record.to_json(&self.captured_by);
        self.db
            .put(|store, writer| Ok(Some(crate::device_kinds::tesla_snapshot(store, &json, writer)?)));
        self.last_poll = Some(LastPoll::succeeded(record.received));
        self.latest = Some(record);
    }

    pub fn failed(&mut self, time: DateTime<Utc>, error: String) {
        self.last_poll = Some(LastPoll::failed(self.last_poll.as_ref(), time, error));
    }

    pub fn latest(&self) -> Option<&Record> {
        self.latest.as_ref()
    }

    pub fn last_poll(&self) -> Option<&LastPoll> {
        self.last_poll.as_ref()
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}
