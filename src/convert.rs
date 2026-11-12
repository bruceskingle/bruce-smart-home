//! Converting a profile made before the store, see store.rs, when it is upgraded to 0.5.0, see
//! profile::upgrade_profile: what each module, and the app itself, kept in the cache directory is put into the profile's
//! site's store, and left where it was. A module converts its own,
//! see ModuleRegistration::convert; the DataSets of the Octopus module import their own files the first time each is
//! used, see data_set::Records::import_legacy.
//!
//! Old files are found by their names: those of a day are `<year>/<month>/<day>#<key>`, e.g. `2026/October/28#status-
//! 134000`, with the month's long name and the day unpadded, see legacy_files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bruce_db::progress::{Progress, prefixed};
use bruce_db::{Item, Micros, Part, Store, Version};
use time::Date;

/// A file in the old layout, see legacy_files.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyFile {
    pub path: PathBuf,
    /// The day a day's file is of, None for one which is not a day's.
    pub date: Option<Date>,
    /// What follows the # in its name, or its whole name if it has none, e.g. status-134000.
    pub key: String,
}

impl LegacyFile {
    pub fn json(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::from_str(&std::fs::read_to_string(&self.path)?)?)
    }

    /// Each line which is JSON, for files of a record a line.
    pub fn lines(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        Ok(std::fs::read_to_string(&self.path)?
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }
}

/// Every file under dir, with the day each is of, see the module documentation.
pub fn legacy_files(dir: &Path) -> Vec<LegacyFile> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(current) = dirs.pop() {
        for entry in std::fs::read_dir(&current).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()).map(str::to_string) else {
                continue;
            };
            let (day, key) = match name.split_once('#') {
                Some((day, key)) => (Some(day.to_string()), key.to_string()),
                None => (None, name.clone()),
            };
            let date = day.and_then(|day| {
                let month = path.parent()?;
                let year = month.parent()?.file_name()?.to_str()?;
                let month = month.file_name()?.to_str()?;
                let format = time::macros::format_description!("[year] [month repr:long] [day padding:none]");
                Date::parse(&format!("{year} {month} {day}"), format).ok()
            });
            files.push(LegacyFile { path, date, key });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Converts the climate readings, `<profile>-climate/<device>/<series>/<date>.json`, each a map of a period's start to
/// its end, value and whether it was open, and the device's name, device.json, into the periods the devices give,
/// see climate.rs.
fn convert_climate(store: &Store, dir: &Path, progress: &mut dyn Progress) -> anyhow::Result<usize> {
    let mut count = 0;
    for device in std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
    {
        progress.check()?;
        progress.report(&format!("device {}", device.file_name().to_string_lossy()));
        let named: serde_json::Value = std::fs::read_to_string(device.path().join("device.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let Some(id) = named.get("id").and_then(|id| id.as_str()).map(str::to_string) else {
            continue;
        };
        let name = named
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_string();
        let mut records = Vec::new();
        for series in std::fs::read_dir(device.path())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_dir())
        {
            let series_name = series.file_name().to_string_lossy().to_string();
            for day in std::fs::read_dir(series.path()).into_iter().flatten().flatten() {
                let periods: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(day.path())
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
                    .unwrap_or_default();
                for (start, saved) in periods {
                    records.push(serde_json::json!({
                        "series": series_name, "start": start, "end": saved["end"], "value": saved["value"],
                        "open": saved["open"], "capturedBy": name,
                    }));
                }
            }
        }
        count += crate::climate::save(store, &id, &records)?;
    }
    Ok(count)
}

/// Converts the poll log, `<profile>-polling/<date>.jsonl`, a record a line, each with its time, see polling.rs.
fn convert_polling(store: &Store, dir: &Path, progress: &mut dyn Progress) -> anyhow::Result<usize> {
    let hk = store.hash_key("app.poll", &[Part::Name(crate::store::origin())])?;
    let mut count = 0;
    let files = legacy_files(dir);
    for (index, file) in files.iter().enumerate() {
        step(progress, index, files.len())?;
        for record in file.lines()? {
            let Some(time) = record
                .get("time")
                .and_then(|time| time.as_str())
                .and_then(|time| Micros::parse(time).ok())
            else {
                continue;
            };
            store.put(&Item::new(
                &hk,
                &time.to_text(),
                Version::new(time, &crate::store::origin()),
                record,
            ))?;
            count += 1;
        }
    }
    Ok(count)
}

/// How long since started, e.g. 42 s, to say how long each part of a conversion took.
fn took(started: std::time::Instant) -> String {
    format!("{:.0} s", started.elapsed().as_secs_f64())
}

/// Before a converter's next file: stops, with Cancelled, if the conversion has been cancelled, which is safe between
/// files, see convert_profile, and otherwise says how far through its files it is, e.g. "1200 of 4839 files", see
/// report_count.
pub fn step(progress: &mut dyn Progress, index: usize, total: usize) -> anyhow::Result<()> {
    progress.check()?;
    bruce_db::progress::report_count(progress, index, total, "files");
    Ok(())
}

/// Converts the profile's old cache into its store, see the module documentation, saying how far it is with progress,
/// e.g. "Converting tesla, 3 of 6: 1200 of 4839 files", and what each part converted.
///
/// It stops if progress is cancelled, between files, or if a part fails, returning the error, without marking the
/// profile converted, so that converting it again, e.g. by upgrading it again, does it all: the old files are as they
/// were, and records already put are put again, which changes nothing.
pub fn convert_profile(profile_name: &str, progress: &mut dyn Progress) -> anyhow::Result<()> {
    let store = crate::store::for_profile(profile_name)?;
    // Once, see CONVERTED.
    let converted = store.hash_key(CONVERTED, &[])?;
    if store.get(&converted, "-")?.is_some() {
        progress.report(&format!(
            "{profile_name}'s data has been converted into the store already"
        ));
        return Ok(());
    }
    // Synced once at the end, as it is many records at once, and synced as usual again however it ends.
    store.set_sync(false);
    let result = convert_parts(&store, profile_name, progress);
    store.set_sync(true);
    store.sync()?;
    result?;
    store.put(&Item::new(
        &converted,
        "-",
        crate::store::version_now(),
        serde_json::json!({ "profile": profile_name }),
    ))?;
    progress.report("Compacting the store");
    let started = std::time::Instant::now();
    let tomorrow = bruce_db::Micros::now()
        .date()?
        .next_day()
        .ok_or_else(|| anyhow::anyhow!("No day after today"))?;
    store.compact_with_progress(tomorrow, &mut prefixed(progress, "Compacting the store: "))?;
    progress.report(&format!("Compacted the store in {}", took(started)));
    progress.report("The Octopus module's other data is imported the first time each part of it is shown");
    Ok(())
}

/// Converts each part of the profile's old cache, the app's own and each module's, see convert_profile, stopping at
/// the first which fails or is cancelled.
fn convert_parts(store: &Arc<Store>, profile_name: &str, progress: &mut dyn Progress) -> anyhow::Result<()> {
    let cache = crate::AppContext::get_cache_dir_path()?;
    let dir = |name: &str| cache.join(format!("{profile_name}-{name}"));
    // Those with a directory to convert, numbered for the progress.
    let converters = crate::ModuleRegistrations::converters();
    let parts: Vec<String> = ["climate", "polling"]
        .iter()
        .map(|part| part.to_string())
        .chain(converters.iter().map(|(module_id, _)| module_id.clone()))
        .filter(|part| dir(part).exists())
        .collect();
    for (index, part) in parts.iter().enumerate() {
        let step = format!("Converting {part}, {} of {}", index + 1, parts.len());
        let started = std::time::Instant::now();
        progress.report(&step);
        let mut report = prefixed(progress, &format!("{step}: "));
        let summary = match part.as_str() {
            "climate" => convert_climate(store, &dir(part), &mut report).map(|count| format!("{count} periods")),
            "polling" => convert_polling(store, &dir(part), &mut report).map(|count| format!("{count} records")),
            module_id => match converters.iter().find(|(id, _)| id == module_id) {
                Some((_, convert)) => convert(store, &dir(part), &mut report),
                None => continue,
            },
        };
        let summary = summary.map_err(|error| error.context(format!("Unable to convert {part}")))?;
        progress.report(&format!("{part}: {summary}, in {}", took(started)));
    }
    Ok(())
}

/// Upgrades the profile, see profile::upgrade_profile, for --upgrade, saying what it did.
pub fn upgrade(profile_name: &str) -> anyhow::Result<()> {
    crate::profile::upgrade_profile(
        profile_name,
        &mut bruce_db::progress::Reporter::new(|message: &str| println!("{message}")),
    )?;
    println!("{profile_name} is now version {}", crate::profile::CURRENT_VERSION);
    Ok(())
}

/// Encrypts the secrets of a profile, for --encryptsecrets, temporary until 0.5.0 is released, see
/// profile::encrypt_secrets.
pub fn encrypt_secrets(profile_name: &str) -> anyhow::Result<()> {
    crate::profile::encrypt_secrets(profile_name)?;
    println!("{profile_name}'s secrets are encrypted");
    Ok(())
}

/// The kind of the record which says a profile's data has been converted, so that it is converted once.
const CONVERTED: &str = "app.converted";

/// A module's converter, see ModuleRegistration::convert. It says how far it is to its last argument, and stops if it
/// is cancelled, between files, see step.
pub type Converter = fn(&Arc<Store>, &Path, &mut dyn Progress) -> anyhow::Result<String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_days_file_is_known_by_its_path() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-convert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("2026/November")).unwrap();
        std::fs::write(dir.join("2026/November/1#status-134000"), "{}").unwrap();
        std::fs::write(dir.join("currentuser"), "{}").unwrap();
        let files = legacy_files(&dir);
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].date, files[0].key.as_str()),
            (Some(time::macros::date!(2026 - 11 - 01)), "status-134000")
        );
        assert_eq!((files[1].date, files[1].key.as_str()), (None, "currentuser"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
