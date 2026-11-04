//! Snapshots of what the gateway says, each the responses of its LIVE endpoints at one time, in a cache file, and once a
//! day the details which rarely change, from its DAILY endpoints, in another.
//!
//! The gateway only says what is happening now, but its meters' energy totals only ever increase, so the energy of
//! any time between two snapshots is the difference between them.

use crate::power::measured::SiteRegisters;
use anyhow::anyhow;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CacheConfig;

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
const DAILY: &[&str] = &[
    "site_info",
    "powerwalls",
    "meters/site",
    "meters/solar",
];

/// A snapshot in a file of its own, e.g. 2026/October/28#snapshot-134054, as they were saved until there were too many
/// for that, which are still read.
const SNAPSHOT_KEY: &str = "snapshot-";
/// A day's snapshots, one a line, in the order they were saved, e.g. 2026/October/28#snapshots, see Snapshots.
const DAY_KEY: &str = "snapshots";
const DETAILS_KEY: &str = "details-";
/// How many lines at the end of a day's file are read to find its latest snapshot, which is usually the last, but a
/// capture device's may be saved after the app's own, see latest.
const LATEST_LINES: usize = 50;

/// The responses of the endpoints, as {"slot": ..., "requested": ..., "received": ..., "responses": {endpoint:
/// response}}. slot is the time the poll was scheduled for, see polling::next_slot, if it was, requested when the first
/// request was sent, and received when the last response arrived. An endpoint which fails is recorded as
/// {"error": "..."}, unless the login fails, which stops it.
async fn fetch(client: &LocalClient, endpoints: &[&str], slot: Option<OffsetDateTime>, progress: &mut dyn FnMut(String)) -> anyhow::Result<serde_json::Value> {
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
            },
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
    ["slot", "requested", "time"].iter()
        .find_map(|name| json[*name].as_str())
        .and_then(|time| OffsetDateTime::parse(time, &Rfc3339).ok())
}

/// The cache file for the time json is of, see time_of, e.g. 2026/October/28#snapshot-134054
fn path_for(cache_config: &CacheConfig, key: &str, json: &serde_json::Value) -> anyhow::Result<std::path::PathBuf> {
    let now = time_of(json).unwrap_or_else(crate::clock::now_utc);
    let time = now.time();
    let key = format!("{key}{:02}{:02}{:02}", time.hour(), time.minute(), time.second());
    cache_config.path_for_day(&key, &now.date().into())
}

/// Saves json in its cache file, see path_for.
fn save(cache_config: &CacheConfig, key: &str, json: &serde_json::Value) -> anyhow::Result<()> {
    let path = path_for(cache_config, key, json)?;
    // Compact, one record a line, as the Octopus cache files are, which Cache Files shows pretty printed.
    serde_json::to_writer(crate::private_file::create_private_file(&path)?, json)?;
    Ok(())
}

/// The snapshots, a file a day, see DAY_KEY, which may be every 10 seconds, too many for a file each, and which of
/// their times have been saved, by day, to save each only once, whether the app or a capture device took it.
#[derive(Default)]
pub struct Snapshots {
    saved: std::sync::Mutex<std::collections::HashMap<time::Date, std::collections::HashSet<i64>>>,
}

impl Snapshots {
    /// Saves a snapshot, unless there is one of its time, to the second, already, e.g. the app's own, which has more in
    /// it than a capture device's, returning whether it was saved.
    pub fn save(&self, cache_config: &CacheConfig, json: &serde_json::Value) -> anyhow::Result<bool> {
        use std::io::Write;

        let time = time_of(json).ok_or_else(|| anyhow!("the snapshot has no time"))?;
        let mut saved = self.saved.lock().unwrap();
        let times = saved.entry(time.date()).or_insert_with(|| times_of_day(cache_config, time.date()));
        if !times.insert(time.unix_timestamp()) {
            return Ok(false);
        }
        let path = cache_config.path_for_day(DAY_KEY, &time.date().into())?;
        let mut line = serde_json::to_vec(json)?;
        line.push(b'\n');
        crate::private_file::append_private_file(&path)?.write_all(&line)?;
        Ok(true)
    }
}

/// The times, to the second, of the snapshots of a day, in its file and in files of their own.
fn times_of_day(cache_config: &CacheConfig, date: time::Date) -> std::collections::HashSet<i64> {
    let mut times: std::collections::HashSet<i64> = read_lines(&cache_config.existing_path_for_day(DAY_KEY, &date.into()))
        .iter()
        .filter_map(|line| time_of(&serde_json::from_str(line).ok()?))
        .map(|time| time.unix_timestamp())
        .collect();
    times.extend(cache_config.files_with_key(SNAPSHOT_KEY).iter()
        .filter_map(|path| path_time(path))
        .filter(|time| time.date() == date)
        .map(|time| time.unix_timestamp()));
    times
}

/// The lines of a day's file, none if it cannot be read.
fn read_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path).map(|text| text.lines().map(String::from).collect()).unwrap_or_default()
}

/// The day files, newest first, with the day each is of.
fn day_files(cache_config: &CacheConfig) -> Vec<(time::Date, std::path::PathBuf)> {
    let mut files: Vec<(time::Date, std::path::PathBuf)> = cache_config.files_with_key(DAY_KEY).into_iter()
        .filter_map(|path| Some((path_date(&path)?, path)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
}

/// The day a day file is of, from its path, e.g. 2026/October/28#snapshots.
fn path_date(path: &std::path::Path) -> Option<time::Date> {
    let name = path.file_name()?.to_str()?;
    let day = name.strip_suffix(&format!("#{DAY_KEY}"))?;
    let month = path.parent()?;
    let year = month.parent()?.file_name()?.to_str()?;
    let month = month.file_name()?.to_str()?;
    // The day is not padded, e.g. 1, see path_for.
    let format = time::macros::format_description!("[year] [month repr:long] [day padding:none]");
    time::Date::parse(&format!("{year} {month} {day}"), format).ok()
}

/// The gateway's software version, in the status response.
fn version(json: &serde_json::Value) -> Option<&str> {
    json.pointer("/responses/status/version").and_then(|version| version.as_str())
}

/// Takes a snapshot, and the details too if they have not been fetched today or the gateway's software has changed,
/// saying what it did with progress. slot is the time the poll was scheduled for, if it was, see Module::poll.
pub async fn take(client: &LocalClient, cache_config: &CacheConfig, snapshots: &Snapshots, slot: Option<OffsetDateTime>,
    progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
    if cache_config.offline {
        return Err(anyhow!("This profile is offline, so nothing can be fetched"));
    }
    let now = crate::clock::now_utc();
    let snapshot = fetch(client, LIVE, slot, progress).await?;
    snapshots.save(cache_config, &snapshot)?;
    progress(format!("Saved the snapshot of {}", time_of(&snapshot).map(crate::util::local_date_time).unwrap_or_default()));

    let details = latest_file(cache_config, DETAILS_KEY);
    let fetched_today = details.as_ref()
        .and_then(time_of)
        .is_some_and(|time| time.date() == now.date());
    let same_version = details.as_ref().is_some_and(|details| version(details) == version(&snapshot));
    if !fetched_today || !same_version {
        let mut details = fetch(client, DAILY, slot, progress).await?;
        // So that the next snapshot can tell whether the software has changed since.
        details["responses"]["status"] = snapshot["responses"]["status"].clone();
        save(cache_config, DETAILS_KEY, &details)?;
        progress("Saved the gateway's details".to_string());
    }
    Ok(())
}

/// The latest snapshot, if there is one: the latest of the end of the latest day's file, and of those in files of their
/// own.
pub fn latest(cache_config: &CacheConfig) -> Option<serde_json::Value> {
    let in_day = day_files(cache_config).first().and_then(|(_, path)| {
        let lines = read_lines(path);
        lines[lines.len().saturating_sub(LATEST_LINES)..].iter()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .max_by_key(time_of)
    });
    let own_file = latest_file(cache_config, SNAPSHOT_KEY);
    [in_day, own_file].into_iter().flatten().max_by_key(time_of)
}

/// The latest cache file whose key starts with key.
fn latest_file(cache_config: &CacheConfig, key: &str) -> Option<serde_json::Value> {
    cache_config.latest_json(key)
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
    let Some(aggregates) = snapshot.pointer("/responses/meters~1aggregates").and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    aggregates.iter()
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
        let text = |pointer: &str| snapshot.pointer(pointer).and_then(|value| value.as_str()).map(String::from);
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

/// The time a snapshot's cache file is for, from its path, e.g. 2026/October/28#snapshot-134054, see path_for.
fn path_time(path: &std::path::Path) -> Option<OffsetDateTime> {
    let name = path.file_name()?.to_str()?;
    let (day, time) = name.split_once(&format!("#{SNAPSHOT_KEY}"))?;
    let month = path.parent()?;
    let year = month.parent()?.file_name()?.to_str()?;
    let month = month.file_name()?.to_str()?;
    let text = format!("{year} {month} {day} {time}");
    let format = time::macros::format_description!("[year] [month repr:long] [day padding:none] [hour][minute][second]");
    Some(time::PrimitiveDateTime::parse(&text, format).ok()?.assume_utc())
}

/// The latest limit snapshots, newest first, by the times they are of, not when they were saved, as a capture device's
/// are saved when it is read, see capture.rs: from the ends of the latest days' files, and files of their own.
pub fn samples(cache_config: &CacheConfig, limit: usize) -> Vec<Sample> {
    let mut snapshots: Vec<serde_json::Value> = Vec::new();
    for (_, path) in day_files(cache_config) {
        let lines = read_lines(&path);
        // A few more than limit, as they may not be in order, see latest.
        snapshots.extend(lines[lines.len().saturating_sub(limit + LATEST_LINES)..].iter()
            .filter_map(|line| serde_json::from_str(line).ok()));
        if snapshots.len() >= limit {
            break;
        }
    }
    let mut paths: Vec<_> = cache_config.files_with_key(SNAPSHOT_KEY).into_iter()
        .map(|path| (path_time(&path), path))
        .collect();
    paths.sort_by(|a, b| b.0.cmp(&a.0));
    snapshots.extend(paths.iter().take(limit).filter_map(|(_, path)| CacheConfig::read_json(path)));
    snapshots.sort_by(|a, b| time_of(b).cmp(&time_of(a)));
    snapshots.iter().take(limit).map(Sample::from_snapshot).collect()
}

/// The registers of the snapshots in the day files from start to end, unix timestamps, in time order, see SiteRegisters.
/// Only the day files, as snapshots in files of their own are minutes apart, too far apart to be used.
pub fn registers(cache_config: &CacheConfig, start: i64, end: i64) -> Vec<SiteRegisters> {
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

    let (Ok(first), Ok(last)) = (OffsetDateTime::from_unix_timestamp(start), OffsetDateTime::from_unix_timestamp(end)) else {
        return Vec::new();
    };
    let mut registers = Vec::new();
    let mut day = first.date();
    while day <= last.date() {
        for line in read_lines(&cache_config.existing_path_for_day(DAY_KEY, &day.into())) {
            let Some(aggregates) = serde_json::from_str::<Line>(&line).ok().and_then(|line| line.responses?.aggregates) else {
                continue;
            };
            let (Some(site), Some(battery), Some(solar), Some(load)) = (aggregates.site, aggregates.battery, aggregates.solar, aggregates.load) else {
                continue;
            };
            let Some(time) = site.last_communication_time.as_deref()
                .and_then(|time| OffsetDateTime::parse(time, &Rfc3339).ok())
                .map(|time| time.unix_timestamp_nanos() as f64 / 1e9)
            else {
                continue;
            };
            if (start as f64..=end as f64).contains(&time) {
                registers.push(SiteRegisters {
                    time,
                    site_imported: site.energy_imported,
                    site_exported: site.energy_exported,
                    battery_imported: battery.energy_imported,
                    solar_exported: solar.energy_exported,
                    load_imported: load.energy_imported,
                    site_power: site.instant_power,
                    // The gateway's is negative while they charge.
                    battery_power: -battery.instant_power,
                    solar_power: solar.instant_power,
                });
            }
        }
        let Some(next) = day.next_day() else { break };
        day = next;
    }
    registers.sort_by(|a, b| a.time.total_cmp(&b.time));
    // The same reading saved twice, e.g. by the app and a capture device, or the gateway not having read again.
    registers.dedup_by(|a, b| a.time == b.time);
    registers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_saved_a_day_a_file_once_each_and_read_newest_first() {
        let dir = std::env::temp_dir().join(format!("marco-sparko-snapshots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache_config = CacheConfig::new(dir.clone(), false, false);
        let snapshot = |time: &str, power: f64| serde_json::json!({
            "slot": time,
            "responses": {"meters/aggregates": {"site": {"instant_power": power, "energy_imported": 1000.0, "energy_exported": 0.0}}},
        });
        // One saved before day files, in a file of its own.
        let old = snapshot("2026-10-29T10:00:00Z", 1.0);
        save(&cache_config, SNAPSHOT_KEY, &old).unwrap();

        let snapshots = Snapshots::default();
        assert!(!snapshots.save(&cache_config, &old).unwrap(), "already in a file of its own");
        assert!(snapshots.save(&cache_config, &snapshot("2026-10-29T10:00:20Z", 3.0)).unwrap());
        // Out of order, as a capture device's may be saved after the app's own.
        assert!(snapshots.save(&cache_config, &snapshot("2026-10-29T10:00:10Z", 2.0)).unwrap());
        assert!(!snapshots.save(&cache_config, &snapshot("2026-10-29T10:00:10Z", 9.0)).unwrap(), "a second of the same time");

        let lines = read_lines(&cache_config.existing_path_for_day(DAY_KEY, &time::macros::date!(2026-10-29).into()));
        assert_eq!(lines.len(), 2);
        assert_eq!(latest(&cache_config).as_ref().and_then(time_of), OffsetDateTime::parse("2026-10-29T10:00:20Z", &Rfc3339).ok());
        let powers: Vec<f64> = samples(&cache_config, 10).iter().filter_map(|sample| Some(sample.meter("site")?.power)).collect();
        assert_eq!(powers, [3.0, 2.0, 1.0]);
        // Another run of the app knows what was saved.
        assert!(!Snapshots::default().save(&cache_config, &snapshot("2026-10-29T10:00:20Z", 3.0)).unwrap());
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
        assert_eq!(sample.time, OffsetDateTime::parse("2026-10-29T07:30:00Z", &Rfc3339).ok());
        assert_eq!(sample.meter("site").map(|meter| meter.imported), Some(44128.49336542916));
        assert_eq!(sample.charge, Some(100.0));
        assert_eq!(sample.grid_status.as_deref(), Some("SystemGridConnected"));
        assert_eq!(sample.reserve, None);
        assert_eq!(sample.captured_by.as_deref(), Some("marco-sparko-capture 0.1.0"));
    }

    #[test]
    fn the_time_is_read_from_the_path() {
        let path = std::path::Path::new("/cache/n2-tesla/2026/October/29#snapshot-111151");
        assert_eq!(path_time(path), OffsetDateTime::parse("2026-10-29T11:11:51Z", &Rfc3339).ok());
        let path = std::path::Path::new("/cache/n2-tesla/2026/November/1#snapshot-111151");
        assert_eq!(path_time(path), OffsetDateTime::parse("2026-11-01T11:11:51Z", &Rfc3339).ok());
    }

    #[test]
    fn the_day_is_read_from_the_path_of_a_day_file() {
        let day = |path: &str| path_date(std::path::Path::new(path));
        assert_eq!(day("/cache/n2-tesla/2026/October/29#snapshots"), Some(time::macros::date!(2026-10-29)));
        assert_eq!(day("/cache/n2-tesla/2026/November/1#snapshots"), Some(time::macros::date!(2026-11-01)));
    }

    #[test]
    fn meters_are_read_from_the_aggregates() {
        let snapshot = serde_json::json!({"responses": {"meters/aggregates": {
            "battery": {"instant_power": 600, "energy_imported": 20567877, "energy_exported": 18241742},
            "error": "not a meter",
        }}});
        assert_eq!(meters(&snapshot), vec![
            Meter { name: "battery".to_string(), power: 600.0, imported: 20567.877, exported: 18241.742 },
        ]);
    }
}
