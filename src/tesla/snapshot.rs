//! Snapshots of what the gateway says, each the responses of its LIVE endpoints at one time, and once a day the details
//! which rarely change, from its DAILY endpoints, kept in the profile's store, see store.rs, as the kinds
//! `tesla.snapshot` and `tesla.details`: a DataSet each for the site's gateway, of each as it was taken, by its time to
//! the second, versioned by that time and written by what took it, the app or a capture device. The app's own is a
//! microsecond newer, so that it wins over a device's of the same second, as it has more in it.
//!
//! The gateway only says what is happening now, but its meters' energy totals only ever increase, so the energy of
//! any time between two snapshots is the difference between them.

use crate::power::measured::SiteRegisters;
use anyhow::anyhow;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use bruce_db::{Flow, Item, Micros, Part, Put, Store, Version};

use super::local::LocalClient;

/// What is fetched for every snapshot, see local::ENDPOINTS. status is small, and has the gateway's software version.
const LIVE: &[&str] = &[
    "status",
    "meters/aggregates",
    "system_status",
    "system_status/soe",
    "system_status/grid_status",
    "operation",
];

/// What rarely changes, so is fetched once a day, or again when the gateway's software version changes.
const DAILY: &[&str] = &["site_info", "powerwalls", "meters/site", "meters/solar"];

/// The kinds of the snapshots and details, see the module documentation, and the part of their hash keys naming the
/// gateway, as a site has one.
const KIND: &str = crate::device_kinds::TESLA_SNAPSHOT;
const DETAILS_KIND: &str = "tesla.details";
const GATEWAY: &str = crate::device_kinds::TESLA_GATEWAY;

/// The responses of the endpoints, as {"slot": ..., "requested": ..., "received": ..., "responses": {endpoint:
/// response}}. slot is the time the poll was scheduled for, see polling::next_slot, if it was, requested when the first
/// request was sent, and received when the last response arrived. An endpoint which fails is recorded as
/// {"error": "..."}, unless the login fails, which stops it.
async fn fetch(
    client: &LocalClient,
    endpoints: &[&str],
    slot: Option<OffsetDateTime>,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<serde_json::Value> {
    let requested = crate::clock::now_utc();
    let mut responses = serde_json::Map::new();
    for endpoint in endpoints {
        progress(format!("Fetching {endpoint}"));
        let response = match client.get(endpoint).await {
            Ok(json) => json,
            Err(error) if error.is::<super::local::PasswordRejected>() => return Err(error),
            Err(error) => {
                crate::log_warning!("Unable to fetch {endpoint} from the Tesla gateway, {error:#}");
                serde_json::json!({ "error": format!("{error:#}") })
            }
        };
        responses.insert(endpoint.to_string(), response);
    }
    let received = crate::clock::now_utc();
    Ok(serde_json::json!({
        "slot": slot.map(|slot| slot.format(&Rfc3339)).transpose()?,
        "requested": requested.format(&Rfc3339)?,
        "received": received.format(&Rfc3339)?,
        "responses": responses,
    }))
}

/// When a snapshot or details are of: the slot of a scheduled poll, otherwise when it was requested, or for one saved
/// before those were recorded its time.
pub fn time_of(json: &serde_json::Value) -> Option<OffsetDateTime> {
    ["slot", "requested", "time"]
        .iter()
        .find_map(|name| json[*name].as_str())
        .and_then(|time| OffsetDateTime::parse(time, &Rfc3339).ok())
}

fn snapshot_key(store: &Store) -> anyhow::Result<String> {
    store.hash_key(KIND, &[Part::from(GATEWAY)])
}

fn details_key(store: &Store) -> anyhow::Result<String> {
    store.hash_key(DETAILS_KIND, &[Part::from(GATEWAY)])
}

/// The sort key of a time, to the second, e.g. 2026-11-04T07:17:10Z.
fn sort_key(time: OffsetDateTime) -> String {
    let format = time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    time.to_offset(time::UtcOffset::UTC).format(format).unwrap_or_default()
}

/// Saves json, a snapshot or details, of the DataSet hk, see the module documentation, taken by the capture device
/// device, or by the app if it is None, returning whether it was new.
fn save_to(store: &Store, hk: &str, json: &serde_json::Value, device: Option<&str>) -> anyhow::Result<bool> {
    let time = time_of(json).ok_or_else(|| anyhow!("the snapshot has no time"))?;
    let sk = sort_key(time);
    let second = Micros::from_datetime(OffsetDateTime::parse(&sk, &Rfc3339)?);
    let version = match device {
        Some(device) => Version::new(second, &crate::store::device_part(device)),
        None => Version::new(Micros(second.0 + 1), &crate::store::origin()),
    };
    Ok(store.put(&Item::new(hk, &sk, version, json.clone()))? == Put::Stored)
}

/// Saves a snapshot taken by the capture device device, or by the app if it is None, as a device writes it, see
/// device_kinds.rs.
pub fn save(store: &Store, json: &serde_json::Value, device: Option<&str>) -> anyhow::Result<bool> {
    let origin = crate::store::origin();
    let writer = device.map_or(
        crate::device_kinds::Writer::App(&origin),
        crate::device_kinds::Writer::Device,
    );
    Ok(store.put(&crate::device_kinds::tesla_snapshot(store, json, writer)?)? == Put::Stored)
}

/// The gateway's software version, in the status response.
fn version(json: &serde_json::Value) -> Option<&str> {
    json.pointer("/responses/status/version")
        .and_then(|version| version.as_str())
}

/// Takes a snapshot, and the details too if they have not been fetched today or the gateway's software has changed,
/// saying what it did with progress. slot is the time the poll was scheduled for, if it was, see Module::poll.
pub async fn take(
    client: &LocalClient,
    store: &Store,
    offline: bool,
    slot: Option<OffsetDateTime>,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<()> {
    if offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    let now = crate::clock::now_utc();
    let snapshot = fetch(client, LIVE, slot, progress).await?;
    save(store, &snapshot, None)?;
    progress(format!(
        "Saved the snapshot of {}",
        time_of(&snapshot).map(crate::util::local_date_time).unwrap_or_default()
    ));

    let details = last(store, &details_key(store)?);
    let fetched_today = details
        .as_ref()
        .and_then(time_of)
        .is_some_and(|time| time.date() == now.date());
    let same_version = details
        .as_ref()
        .is_some_and(|details| version(details) == version(&snapshot));
    if !fetched_today || !same_version {
        let mut details = fetch(client, DAILY, slot, progress).await?;
        // So that the next snapshot can tell whether the software has changed since.
        details["responses"]["status"] = snapshot["responses"]["status"].clone();
        save_to(store, &details_key(store)?, &details, None)?;
        progress("Saved the gateway's details".to_string());
    }
    Ok(())
}

/// The last record of the DataSet hk, looking first at the last two days, as a snapshot is usually recent, then at
/// them all.
fn last(store: &Store, hk: &str) -> Option<serde_json::Value> {
    let recent = sort_key(crate::clock::now_utc() - time::Duration::days(2));
    let mut found = None;
    let _ = store.query(hk, recent.as_str()..bruce_db::ALL.end, &mut |item| {
        found = item.attributes;
        Ok(Flow::Continue)
    });
    if found.is_none() {
        let _ = store.query(hk, bruce_db::ALL, &mut |item| {
            found = item.attributes;
            Ok(Flow::Continue)
        });
    }
    found
}

/// The latest snapshot, if there is one.
pub fn latest(store: &Store) -> Option<serde_json::Value> {
    last(store, &snapshot_key(store).ok()?)
}

/// One meter of meters/aggregates, e.g. the battery, with power in W and energy in kWh.
#[derive(Debug, Clone, PartialEq)]
pub struct Meter {
    pub name: String,
    pub power: f64,
    pub imported: f64,
    pub exported: f64,
}

/// The meters of a snapshot's meters/aggregates response, in which energy is in Wh.
pub fn meters(snapshot: &serde_json::Value) -> Vec<Meter> {
    let Some(aggregates) = snapshot
        .pointer("/responses/meters~1aggregates")
        .and_then(|value| value.as_object())
    else {
        return Vec::new();
    };
    aggregates
        .iter()
        .filter(|(_, meter)| meter.get("instant_power").is_some())
        .map(|(name, meter)| {
            let number = |field: &str| meter.get(field).and_then(|value| value.as_f64()).unwrap_or_default();
            Meter {
                name: name.clone(),
                power: number("instant_power"),
                imported: number("energy_imported") / 1000.0,
                exported: number("energy_exported") / 1000.0,
            }
        })
        .collect()
}

/// What one snapshot says, for the Samples page.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub time: Option<OffsetDateTime>,
    pub meters: Vec<Meter>,
    /// The battery's charge, as the gateway says it, which the Tesla app shows less the 5% it keeps in reserve.
    pub charge: Option<f64>,
    /// The backup reserve, as the gateway says it, which the Tesla app shows less 5%.
    pub reserve: Option<f64>,
    pub mode: Option<String>,
    pub grid_status: Option<String>,
    /// What took the snapshot, e.g. marco-sparko-capture 0.1.0, None if the app did, see capture.rs.
    pub captured_by: Option<String>,
}

impl Sample {
    fn from_snapshot(snapshot: &serde_json::Value) -> Sample {
        let text = |pointer: &str| {
            snapshot
                .pointer(pointer)
                .and_then(|value| value.as_str())
                .map(String::from)
        };
        let number = |pointer: &str| snapshot.pointer(pointer).and_then(|value| value.as_f64());
        Sample {
            time: time_of(snapshot),
            meters: meters(snapshot),
            charge: number("/responses/system_status~1soe/percentage"),
            reserve: number("/responses/operation/backup_reserve_percent"),
            mode: text("/responses/operation/real_mode"),
            grid_status: text("/responses/system_status~1grid_status/grid_status"),
            captured_by: text("/capturedBy"),
        }
    }

    /// The meter, e.g. battery, if the snapshot has it.
    pub fn meter(&self, name: &str) -> Option<&Meter> {
        self.meters.iter().find(|meter| meter.name == name)
    }
}

/// The latest limit snapshots, newest first, by the times they are of: of the last day, or of the last week if there
/// are fewer.
pub fn samples(store: &Store, limit: usize) -> Vec<Sample> {
    let Ok(hk) = snapshot_key(store) else { return Vec::new() };
    let mut snapshots: Vec<serde_json::Value> = Vec::new();
    for days in [1, 7] {
        let from = sort_key(crate::clock::now_utc() - time::Duration::days(days));
        snapshots.clear();
        let _ = store.query(&hk, from.as_str()..bruce_db::ALL.end, &mut |item| {
            snapshots.extend(item.attributes);
            Ok(Flow::Continue)
        });
        if snapshots.len() >= limit {
            break;
        }
    }
    snapshots.iter().rev().take(limit).map(Sample::from_snapshot).collect()
}

/// The registers of the snapshots from start to end, unix timestamps, in time order, see SiteRegisters.
pub fn registers(store: &Store, start: i64, end: i64) -> Vec<SiteRegisters> {
    // Only what is needed, as there are thousands a day.
    #[derive(serde::Deserialize)]
    struct Line {
        responses: Option<Responses>,
    }
    #[derive(serde::Deserialize)]
    struct Responses {
        #[serde(rename = "meters/aggregates")]
        aggregates: Option<Aggregates>,
    }
    #[derive(serde::Deserialize)]
    struct Aggregates {
        site: Option<Meter>,
        battery: Option<Meter>,
        solar: Option<Meter>,
        load: Option<Meter>,
    }
    #[derive(serde::Deserialize)]
    struct Meter {
        energy_imported: f64,
        energy_exported: f64,
        instant_power: f64,
        last_communication_time: Option<String>,
    }

    let (Ok(first), Ok(last)) = (
        OffsetDateTime::from_unix_timestamp(start),
        OffsetDateTime::from_unix_timestamp(end),
    ) else {
        return Vec::new();
    };
    let mut registers = Vec::new();
    let Ok(hk) = snapshot_key(store) else { return registers };
    // The gateway's own time of a reading may be a little before the snapshot's.
    let (from, to) = (
        sort_key(first - time::Duration::minutes(5)),
        sort_key(last + time::Duration::minutes(5)),
    );
    let _ = store.query(&hk, from.as_str()..to.as_str(), &mut |item| {
        let Some(aggregates) = item
            .attributes
            .and_then(|json| serde_json::from_value::<Line>(json).ok())
            .and_then(|line| line.responses?.aggregates)
        else {
            return Ok(Flow::Continue);
        };
        let (Some(site), Some(battery), Some(solar), Some(load)) =
            (aggregates.site, aggregates.battery, aggregates.solar, aggregates.load)
        else {
            return Ok(Flow::Continue);
        };
        let Some(time) = site
            .last_communication_time
            .as_deref()
            .and_then(|time| OffsetDateTime::parse(time, &Rfc3339).ok())
            .map(|time| time.unix_timestamp_nanos() as f64 / 1e9)
        else {
            return Ok(Flow::Continue);
        };
        if (start as f64..=end as f64).contains(&time) {
            registers.push(SiteRegisters {
                time,
                site_imported: site.energy_imported,
                site_exported: site.energy_exported,
                battery_imported: battery.energy_imported,
                battery_exported: battery.energy_exported,
                solar_exported: solar.energy_exported,
                load_imported: load.energy_imported,
                site_power: site.instant_power,
                // The gateway's is negative while they charge.
                battery_power: -battery.instant_power,
                solar_power: solar.instant_power,
            });
        }
        Ok(Flow::Continue)
    });
    registers.sort_by(|a, b| a.time.total_cmp(&b.time));
    // The same reading saved twice, e.g. by the app and a capture device, or the gateway not having read again.
    registers.dedup_by(|a, b| a.time == b.time);
    registers
}

/// Converts the snapshots and details kept in the module's directory of the cache before the store, a day's file of
/// snapshots a line, `<day>#snapshots`, or a file each, `<day>#snapshot-HHMMSS` and `<day>#details-HHMMSS`, see
/// convert.rs. Those a capture device took are kept as the app's, as which device is not known.
pub fn convert_legacy(
    store: &std::sync::Arc<Store>,
    dir: &std::path::Path,
    progress: &mut dyn bruce_db::progress::Progress,
) -> anyhow::Result<String> {
    let (mut snapshots, mut details) = (0, 0);
    let files = crate::convert::legacy_files(dir);
    for (index, file) in files.iter().enumerate() {
        crate::convert::step(progress, index, files.len())?;
        if file.key == "snapshots" {
            for json in file.lines()? {
                snapshots += save(store, &json, None)? as usize;
            }
        } else if file.key.starts_with("snapshot-") {
            snapshots += save(store, &file.json()?, None)? as usize;
        } else if file.key.starts_with("details-") {
            details += save_to(store, &details_key(store)?, &file.json()?, None)? as usize;
        }
    }
    Ok(format!("{snapshots} snapshots, {details} details"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_saved_once_each_and_read_newest_first_the_apps_own_winning() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-snapshots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let site = bruce_db::Id::of(&serde_json::json!({"site": "snapshot test"})).unwrap();
        let store = Store::open(&format!("file://{}", dir.display()), &site).unwrap();
        let now = crate::clock::now_utc().replace_millisecond(0).unwrap();
        let at = |seconds: i64| (now - time::Duration::seconds(seconds)).format(&Rfc3339).unwrap();
        let snapshot = |time: &str, power: f64| {
            serde_json::json!({
                "slot": time,
                "responses": {"meters/aggregates": {"site": {"instant_power": power, "energy_imported": 1000.0, "energy_exported": 0.0}}},
            })
        };
        assert!(save(&store, &snapshot(&at(30), 1.0), Some("44:1B:F6:CC:7F:CC")).unwrap());
        assert!(save(&store, &snapshot(&at(10), 3.0), None).unwrap());
        // Out of order, as a capture device's may be saved after the app's own.
        assert!(save(&store, &snapshot(&at(20), 2.0), Some("44:1B:F6:CC:7F:CC")).unwrap());
        assert!(
            !save(&store, &snapshot(&at(20), 9.0), Some("44:1B:F6:CC:7F:CC")).unwrap(),
            "a second of the same time"
        );
        // The app's own of a second a device has saved replaces it, and a device's of the app's does not.
        assert!(save(&store, &snapshot(&at(30), 4.0), None).unwrap());
        assert!(!save(&store, &snapshot(&at(10), 5.0), Some("44:1B:F6:CC:7F:CC")).unwrap());

        assert_eq!(
            latest(&store).as_ref().and_then(time_of),
            OffsetDateTime::parse(&at(10), &Rfc3339).ok()
        );
        let powers: Vec<f64> = samples(&store, 10)
            .iter()
            .filter_map(|sample| Some(sample.meter("site")?.power))
            .collect();
        assert_eq!(powers, [3.0, 2.0, 4.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_capture_device_record_is_a_sample() {
        // As capture/src/records.rs writes them.
        let record = serde_json::json!({
            "slot": "2026-10-29T07:30:00.000000Z",
            "requested": "2026-10-29T07:30:00.201000Z",
            "received": "2026-10-29T07:30:02.950000Z",
            "capturedBy": "marco-sparko-capture 0.1.0",
            "responses": {
                "meters/aggregates": {"site": {"instant_power": -16.0, "energy_imported": 44128493.36542916,
                    "energy_exported": 13867712.52581578, "last_communication_time": "2026-10-29T07:30:01.015587Z"}},
                "system_status/soe": {"percentage": 100.0},
                "system_status/grid_status": {"grid_status": "SystemGridConnected", "grid_services_active": false},
                "operation": {"error": "the gateway responded 502 to operation: "},
            },
        });
        let sample = Sample::from_snapshot(&record);
        assert_eq!(
            sample.time,
            OffsetDateTime::parse("2026-10-29T07:30:00Z", &Rfc3339).ok()
        );
        assert_eq!(
            sample.meter("site").map(|meter| meter.imported),
            Some(44128.49336542916)
        );
        assert_eq!(sample.charge, Some(100.0));
        assert_eq!(sample.grid_status.as_deref(), Some("SystemGridConnected"));
        assert_eq!(sample.reserve, None);
        assert_eq!(sample.captured_by.as_deref(), Some("marco-sparko-capture 0.1.0"));
    }

    #[test]
    fn meters_are_read_from_the_aggregates() {
        let snapshot = serde_json::json!({"responses": {"meters/aggregates": {
            "battery": {"instant_power": 600, "energy_imported": 20567877, "energy_exported": 18241742},
            "error": "not a meter",
        }}});
        assert_eq!(
            meters(&snapshot),
            vec![Meter {
                name: "battery".to_string(),
                power: 600.0,
                imported: 20567.877,
                exported: 18241.742
            },]
        );
    }
}
