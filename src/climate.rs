//! The temperature and humidity which the profile's capture devices measure themselves, e.g. the Waveshare
//! ESP32-S3-RLCD-4.2's sensor, read from them every 5 minutes, see ClimateReader, and kept for the Temp & Humidity
//! page, see views/climate.rs.
//!
//! Unlike the other records the devices give, which are copies of what a source said, these are the readings
//! themselves, so each device's are kept separately. A device gives them as periods of each series, e.g. 23.4 °C from
//! one time to another, see the capture firmware's climate.rs, and gives the one still open again, with its end moved
//! on, each time it is read, so a period is saved by its start, replacing what was saved of it before.
//!
//! They are kept in the profile's store, see store.rs, as the kind `climate.period`: a DataSet for each device and
//! series, by the device's ID, of each period as the device gave it, by its start, versioned by its end, written by the
//! device, so that a period given again with its end moved on replaces what was kept of it. Where the app has got to in
//! reading a device is a `#read` record in each of its DataSets, and the device's name, as it last said, is in
//! `device.info`.

use std::sync::Arc;
use std::time::Duration;

use bruce_db::{Flow, Item, Micros, Part, Store, Version};
use dioxus::prelude::*;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::capture_device::{CaptureReader, DeviceProfile};
use crate::{Module, PageInfo};

/// The id the reader is polled as, see polling.rs.
pub const MODULE_ID: &str = "climate";

/// How often the devices are read.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// What a device measures, by the name it gives each series, with how it is shown.
pub const SERIES: [Series; 2] = [
    Series { name: "temperature", label: "Temperature", unit: "°C", decimals: 1 },
    Series { name: "humidity", label: "Humidity", unit: "%", decimals: 0 },
];

/// The board's battery's voltage, which a device with one, e.g. the RLCD, gives with the temperature and humidity, but
/// which is about the device, so is shown on its card on the Devices page, not with them.
pub const BATTERY: Series = Series { name: "battery", label: "Battery", unit: "V", decimals: 2 };

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Series {
    pub name: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub decimals: usize,
}

/// The kind of the periods, and of the devices' names, see the module documentation.
const KIND: &str = "climate.period";
const DEVICE_KIND: &str = "device.info";
/// The sort key of where the app has got to in reading a DataSet's device.
const READ: &str = "#read";

/// A period as the page draws it, in unix seconds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Period {
    pub start: i64,
    pub end: i64,
    pub value: f64,
}

/// One device's periods of each of SERIES, in order, see load.
#[derive(Clone, PartialEq, Debug)]
pub struct DeviceClimate {
    pub id: String,
    /// Its name on the network, e.g. rlcd, as it last said, or its ID.
    pub name: String,
    pub series: [Vec<Period>; 2],
}

/// The profile's store, which the readings are kept in, see store.rs.
pub fn store(profile_name: &str) -> anyhow::Result<Arc<Store>> {
    crate::store::for_profile(profile_name)
}

fn series_key(store: &Store, device_id: &str, series: &str) -> anyhow::Result<String> {
    store.hash_key(KIND, &[Part::Name(crate::store::device_part(device_id)), Part::Name(series.to_string())])
}

fn device_key(store: &Store, device_id: &str) -> anyhow::Result<String> {
    store.hash_key(DEVICE_KIND, &[Part::Name(crate::store::device_part(device_id))])
}

fn parse(time: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(time, &Rfc3339).ok()
}

/// A period's version: its end, and a microsecond more once it is closed, so that the period closed replaces the same
/// period open with the same end, written by the device.
fn period_version(device_id: &str, end: OffsetDateTime, open: bool) -> Version {
    let end = Micros::from_datetime(end);
    Version::new(Micros(if open { end.0 } else { end.0 + 1 }), &crate::store::device_part(device_id))
}

/// Saves the records read from device, each as the device gave it, see the module documentation, returning how many
/// were new or moved a period's end on.
pub(crate) fn save(store: &Store, device_id: &str, records: &[Value]) -> anyhow::Result<usize> {
    let mut name = None;
    // Records which are not periods, e.g. another feature's on a damaged SD card, counted, with the first, for one
    // warning, rather than one each, which can be hundreds a read.
    let mut not_periods = (0, None);
    let mut changed = 0;
    let mut latest: std::collections::BTreeMap<String, OffsetDateTime> = std::collections::BTreeMap::new();
    for record in records {
        let field = |key: &str| record.get(key).and_then(Value::as_str);
        let (Some(series), Some(start), Some(end)) = (field("series"), field("start"), field("end").and_then(parse)) else {
            not_periods.0 += 1;
            not_periods.1.get_or_insert(record);
            continue;
        };
        if record.get("value").and_then(Value::as_f64).is_none() || parse(start).is_none() {
            not_periods.0 += 1;
            not_periods.1.get_or_insert(record);
            continue;
        }
        let open = record.get("open").and_then(Value::as_bool).unwrap_or(false);
        let item = Item::new(&series_key(store, device_id, series)?, start, period_version(device_id, end, open), record.clone());
        if store.put(&item)? == bruce_db::Put::Stored {
            changed += 1;
        }
        let latest_end = latest.entry(series.to_string()).or_insert(end);
        *latest_end = (*latest_end).max(end);
        name = field("capturedBy").or(name);
    }
    if let (count, Some(first)) = not_periods {
        crate::log_warning!("{count} of the temperature and humidity records from {device_id} are not periods, e.g. {first}");
    }
    for (series, end) in latest {
        let item = Item::new(&series_key(store, device_id, &series)?, READ, crate::store::version_now(),
            serde_json::json!({ "end": end.format(&Rfc3339)? }));
        if latest_end_of(store, device_id, &series).is_none_or(|read| read < end) {
            store.put(&item)?;
        }
    }
    if let Some(name) = name {
        let known = store.get(&device_key(store, device_id)?, "-")?
            .and_then(|item| item.attributes?.get("name")?.as_str().map(str::to_string));
        if known.as_deref() != Some(name) {
            store.put(&Item::new(&device_key(store, device_id)?, "-", crate::store::version_now(),
                serde_json::json!({ "id": device_id, "name": name })))?;
        }
    }
    Ok(changed)
}

/// The end of the latest period read of a device's series, see READ.
fn latest_end_of(store: &Store, id: &str, series: &str) -> Option<OffsetDateTime> {
    let item = store.get(&series_key(store, id, series).ok()?, READ).ok()??;
    parse(item.attributes?.get("end")?.as_str()?)
}

/// The end of the latest period read of device, of the series it has least of, to read only those after it.
fn latest_end(store: &Store, id: &str) -> Option<OffsetDateTime> {
    SERIES.iter().filter_map(|series| latest_end_of(store, id, series.name)).min()
}

/// The periods of every device which has any, of those saved, which overlap from to to, by its name.
pub fn load(store: &Store, from: OffsetDateTime, to: OffsetDateTime) -> Vec<DeviceClimate> {
    let prefix = format!("{}/{KIND}/", store.site());
    let mut ids: Vec<String> = store.data_sets(&prefix).unwrap_or_default().iter()
        .filter_map(|hk| hk.strip_prefix(&prefix)?.split('/').next().map(str::to_string))
        .collect();
    ids.dedup();
    let mut devices: Vec<DeviceClimate> = ids.into_iter()
        .map(|id| {
            let named = device_key(store, &id).ok().and_then(|key| store.get(&key, "-").ok().flatten()).and_then(|item| item.attributes);
            let name = named.as_ref().and_then(|named| named.get("name")?.as_str()).filter(|name| !name.is_empty()).unwrap_or(&id).to_string();
            let id = named.as_ref().and_then(|named| named.get("id")?.as_str()).unwrap_or(&id).to_string();
            let series = SERIES.map(|series| load_series(store, &id, &series, from, to));
            DeviceClimate { id, name, series }
        })
        .filter(|device| device.series.iter().any(|periods| !periods.is_empty()))
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices
}

/// A device's periods of a series, of those saved, which overlap from to to, in time order.
pub fn load_series(store: &Store, id: &str, series: &Series, from: OffsetDateTime, to: OffsetDateTime) -> Vec<Period> {
    let Ok(hk) = series_key(store, id, series.name) else { return Vec::new() };
    // A period which starts the day before may reach into the range.
    let start = from.date().previous_day().unwrap_or(from.date()).to_string();
    let end = (to + time::Duration::seconds(1)).format(&Rfc3339).unwrap_or_default();
    let mut periods = Vec::new();
    let _ = store.query(&hk, start.as_str()..end.as_str(), &mut |item| {
        let record = item.attributes.unwrap_or_default();
        let (Some(start), Some(end), Some(value)) = (
            record.get("start").and_then(Value::as_str).and_then(parse),
            record.get("end").and_then(Value::as_str).and_then(parse),
            record.get("value").and_then(Value::as_f64),
        ) else { return Ok(Flow::Continue) };
        if end > from && start < to {
            periods.push(Period { start: start.unix_timestamp(), end: end.unix_timestamp(), value });
        }
        Ok(Flow::Continue)
    });
    periods
}

/// Reads the periods from the profile's capture devices which take them, polled as MODULE_ID, see polling.rs.
pub struct ClimateReader {
    profile_name: String,
    capture: CaptureReader,
}

impl ClimateReader {
    pub fn new(profile_name: &str) -> ClimateReader {
        ClimateReader { profile_name: profile_name.to_string(), capture: CaptureReader::new("capture/climate/records") }
    }
}

#[async_trait::async_trait(?Send)]
impl Module for ClimateReader {
    fn get_page_list(&self) -> Vec<PageInfo> {
        Vec::new()
    }

    fn module_id(&self) -> &'static str {
        MODULE_ID
    }

    fn get_component(&self, _page_id: &str, _path: Vec<String>) -> Element {
        rsx! {}
    }

    fn help_topic(&self, _page_id: &str, _path: &[String]) -> String {
        "climate.md".to_string()
    }

    fn cli_debug(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn fetch_latest_data(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(POLL_INTERVAL)
    }

    fn next_poll(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        self.capture.next_read(now, POLL_INTERVAL)
    }

    /// Reads each device's periods since the end of the latest saved of it, its own, as each device's are its own, so
    /// that a device with none, e.g. one without a sensor, does not make the others give all they have, saving them a
    /// device at a time, as reading and writing a day's file for each period would be slow when catching up.
    async fn poll(&self, _slot: Option<OffsetDateTime>) -> anyhow::Result<String> {
        let store = store(&self.profile_name)?;
        let devices = crate::capture_device::adopted_devices(&self.profile_name);
        let mut read: Vec<(DeviceProfile, Vec<Value>)> = Vec::new();
        let takes = self.capture.read_each(&devices, &|device| latest_end(&store, &device.id), &mut |device, record| {
            match read.last_mut() {
                Some((last, records)) if last.id == device.id => records.push(record.clone()),
                _ => read.push((device.clone(), vec![record.clone()])),
            }
            Ok(true)
        }, &mut |_| {}).await?;
        if !takes {
            return Ok("No capture device measures the temperature and humidity".to_string());
        }
        let mut changed = 0;
        let count: usize = read.iter().map(|(_, records)| records.len()).sum();
        for (device, records) in &read {
            changed += save(&store, &device.id, records)?;
        }
        Ok(format!("Read {count} temperature and humidity periods, {changed} new or longer"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_period_read_again_replaces_what_was_saved() {
        let dir = std::env::temp_dir().join(format!("marco-sparko-climate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let site = bruce_db::Id::of(&serde_json::json!({"site": "climate test"})).unwrap();
        let store = Store::open(&format!("file://{}", dir.display()), &site).unwrap();
        let device = DeviceProfile { id: "44:1B:F6:CC:7F:CC".to_string(), address: String::new(), fingerprint: String::new(), key: String::new() };
        let record = |start: &str, end: &str, value: f64, open: bool| serde_json::json!({
            "series": "temperature", "start": start, "end": end, "value": value, "unit": "°C", "open": open, "capturedBy": "rlcd",
        });
        assert_eq!(save(&store, &device.id, &[record("2026-11-04T10:00:00Z", "2026-11-04T10:01:00Z", 23.4, true)]).unwrap(), 1);
        // The open period again, with its end moved on, then closed, and the next one.
        assert_eq!(save(&store, &device.id, &[
            record("2026-11-04T10:00:00Z", "2026-11-04T10:02:00Z", 23.4, true),
            record("2026-11-04T10:00:00Z", "2026-11-04T10:02:30Z", 23.4, false),
            record("2026-11-04T10:02:30Z", "2026-11-04T10:02:30Z", 23.5, true),
        ]).unwrap(), 3);
        // Read again, nothing changes.
        assert_eq!(save(&store, &device.id, &[record("2026-11-04T10:00:00Z", "2026-11-04T10:02:00Z", 23.4, true)]).unwrap(), 0);
        let at = |text: &str| parse(text).unwrap();
        let devices = load(&store, at("2026-11-04T00:00:00Z"), at("2026-11-05T00:00:00Z"));
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "rlcd");
        assert_eq!(devices[0].id, device.id);
        assert_eq!(devices[0].series[0], vec![
            Period { start: at("2026-11-04T10:00:00Z").unix_timestamp(), end: at("2026-11-04T10:02:30Z").unix_timestamp(), value: 23.4 },
            Period { start: at("2026-11-04T10:02:30Z").unix_timestamp(), end: at("2026-11-04T10:02:30Z").unix_timestamp(), value: 23.5 },
        ]);
        assert_eq!(latest_end_of(&store, &device.id, "temperature"), Some(at("2026-11-04T10:02:30Z")));
        // The periods are kept as the device gave them.
        let hk = series_key(&store, &device.id, "temperature").unwrap();
        assert_eq!(store.get(&hk, "2026-11-04T10:00:00Z").unwrap().unwrap().attributes,
            Some(record("2026-11-04T10:00:00Z", "2026-11-04T10:02:30Z", 23.4, false)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
