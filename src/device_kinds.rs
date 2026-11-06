//! The kinds of records a capture device writes, and how each record becomes an item of the store, shared by the app,
//! whose modules write the same kinds when they poll the sources themselves, and the capture firmware, which includes
//! this file with `#[path]`, see capture/src/db.rs, so that a device writes exactly the items the app would, and the
//! two cannot drift, see bruce-db's docs/design.md, "Capture devices on the store".
//!
//! It uses only bruce-db, serde_json and anyhow, as the firmware has none of the app's other crates, and it names no
//! module's code, only the kinds, which the app's modules and the firmware's features both write.

use bruce_db::{Id, Item, Micros, Part, Store, Version};
use serde_json::Value;

/// The gateway's snapshots, a DataSet for the site's gateway, as a site has one, see the app's tesla/snapshot.rs.
pub const TESLA_SNAPSHOT: &str = "tesla.snapshot";
pub const TESLA_GATEWAY: &str = "gateway";
/// The myenergi hub's statuses, a DataSet for the site's hub, see the app's myenergi/fetch.rs.
pub const MYENERGI_STATUS: &str = "myenergi.status";
pub const MYENERGI_HUB: &str = "hub";
/// Octopus's smart charging polls, a DataSet for the account, by the hash of its number, see the app's
/// octopus/flex_poll.rs.
pub const OCTOPUS_FLEX: &str = "octopus.flex";
/// A device's temperature, humidity and battery periods, a DataSet for each device and series, see the app's climate.rs.
pub const CLIMATE_PERIOD: &str = "climate.period";
/// A device's name, a DataSet for each device, with the one record `-`.
pub const DEVICE_INFO: &str = "device.info";

/// Who writes an item: a capture device, by its ID, or the app, by its origin, whose version of a sampled record is a
/// microsecond newer than a device's of the same second, so that it wins, as it has more in it.
#[derive(Debug, Clone, Copy)]
pub enum Writer<'a> {
    Device(&'a str),
    App(&'a str),
}

/// A capture device's part of a hash key, and its origin: its ID, a MAC address, lowercase without colons, e.g.
/// 441bf6cc7fcc.
pub fn device_part(device_id: &str) -> String {
    device_id.replace(':', "").to_lowercase()
}

/// When a record is of: the first of fields which is an RFC 3339 time.
pub fn time_of(record: &Value, fields: &[&str]) -> Option<Micros> {
    fields
        .iter()
        .find_map(|name| record.get(*name)?.as_str())
        .and_then(|time| Micros::parse(time).ok())
}

/// The sort key of a time, to the second, e.g. 2026-11-04T07:17:10Z.
pub fn second_key(time: Micros) -> String {
    let text = second_of(time).to_text();
    format!("{}Z", &text[..19])
}

/// The second a time is in.
fn second_of(time: Micros) -> Micros {
    Micros(time.0.div_euclid(1_000_000) * 1_000_000)
}

/// A record sampled once, e.g. a snapshot, of the DataSet hk, at time, by its second, versioned by that second, see
/// Writer.
pub fn sampled(hk: &str, time: Micros, record: &Value, writer: Writer) -> Item {
    let second = second_of(time);
    let version = match writer {
        Writer::Device(device) => Version::new(second, &device_part(device)),
        Writer::App(origin) => Version::new(Micros(second.0 + 1), origin),
    };
    Item::new(hk, &second_key(second), version, record.clone())
}

pub fn tesla_snapshot_key(store: &Store) -> anyhow::Result<String> {
    store.hash_key(TESLA_SNAPSHOT, &[Part::from(TESLA_GATEWAY)])
}

/// When a Tesla snapshot is of: the slot of a scheduled poll, otherwise when it was requested, or for one saved before
/// those were recorded its time.
pub fn tesla_time(record: &Value) -> Option<Micros> {
    time_of(record, &["slot", "requested", "time"])
}

/// A Tesla snapshot, {"slot", "requested", "received", "responses"}, see the app's tesla/snapshot.rs.
pub fn tesla_snapshot(store: &Store, record: &Value, writer: Writer) -> anyhow::Result<Item> {
    let time = tesla_time(record).ok_or_else(|| anyhow::anyhow!("the snapshot has no time"))?;
    Ok(sampled(&tesla_snapshot_key(store)?, time, record, writer))
}

pub fn myenergi_status_key(store: &Store) -> anyhow::Result<String> {
    store.hash_key(MYENERGI_STATUS, &[Part::from(MYENERGI_HUB)])
}

/// When a myenergi status is of: the slot of a scheduled poll, otherwise when it was requested, None for one saved
/// before those were recorded.
pub fn myenergi_time(record: &Value) -> Option<Micros> {
    time_of(record, &["slot", "requested"])
}

/// A myenergi status, {"slot", "requested", "received", "response"}, see the app's myenergi/fetch.rs.
pub fn myenergi_status(store: &Store, record: &Value, writer: Writer) -> anyhow::Result<Item> {
    let time = myenergi_time(record).ok_or_else(|| anyhow::anyhow!("the status has no time"))?;
    Ok(sampled(&myenergi_status_key(store)?, time, record, writer))
}

/// The account's DataSet of Octopus's smart charging polls, by the hash of its number, see the design's IDs.
pub fn octopus_flex_key(store: &Store, account: &str) -> anyhow::Result<String> {
    let account = Id::of(&serde_json::json!({"octopus": account}))?;
    store.hash_key(OCTOPUS_FLEX, &[Part::Id(account)])
}

/// When an Octopus poll is of: its slot, if it was taken on one, otherwise when it was requested.
pub fn octopus_time(record: &Value) -> Option<Micros> {
    time_of(record, &["slot", "requested", "end"])
}

/// A poll of Octopus's smart charging of the account, {"slot", "requested", "received", "responses"}, see the app's
/// octopus/flex_poll.rs.
pub fn octopus_flex(store: &Store, account: &str, record: &Value, writer: Writer) -> anyhow::Result<Item> {
    let time = octopus_time(record).ok_or_else(|| anyhow::anyhow!("the poll has no time"))?;
    Ok(sampled(&octopus_flex_key(store, account)?, time, record, writer))
}

/// A device's DataSet of a series' periods, e.g. temperature.
pub fn climate_period_key(store: &Store, device_id: &str, series: &str) -> anyhow::Result<String> {
    store.hash_key(CLIMATE_PERIOD, &[Part::Name(device_part(device_id)), Part::Name(series.to_string())])
}

/// A period of a device's temperature, humidity or battery, {"series", "start", "end", "value", "unit", "open",
/// "capturedBy"}, by its start, versioned by its end, and a microsecond more once it is closed, so that the period
/// closed replaces the same period open with the same end. None if the record is not a period, e.g. another feature's
/// on a damaged SD card.
pub fn climate_period(store: &Store, device_id: &str, record: &Value) -> anyhow::Result<Option<Item>> {
    let field = |key: &str| record.get(key).and_then(Value::as_str);
    let (Some(series), Some(start), Some(end)) = (field("series"), field("start"), field("end")) else {
        return Ok(None);
    };
    let (Ok(_), Ok(end)) = (Micros::parse(start), Micros::parse(end)) else {
        return Ok(None);
    };
    if record.get("value").and_then(Value::as_f64).is_none() {
        return Ok(None);
    }
    let open = record.get("open").and_then(Value::as_bool).unwrap_or(false);
    let version = Version::new(Micros(if open { end.0 } else { end.0 + 1 }), &device_part(device_id));
    Ok(Some(Item::new(&climate_period_key(store, device_id, series)?, start, version, record.clone())))
}

pub fn device_info_key(store: &Store, device_id: &str) -> anyhow::Result<String> {
    store.hash_key(DEVICE_INFO, &[Part::Name(device_part(device_id))])
}

/// A device's name, {"id", "name"}, e.g. rlcd, as it says on the network, written by writer at time.
pub fn device_info(store: &Store, device_id: &str, name: &str, time: Micros, writer: Writer) -> anyhow::Result<Item> {
    let origin = match writer {
        Writer::Device(device) => device_part(device),
        Writer::App(origin) => origin.to_string(),
    };
    Ok(Item::new(
        &device_info_key(store, device_id)?,
        "-",
        Version::new(time, &origin),
        serde_json::json!({ "id": device_id, "name": name }),
    ))
}
