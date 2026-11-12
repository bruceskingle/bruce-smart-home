//! A DataSet's records in the profile's store, see store.rs, which the DataSet flavours keep their data in, in place
//! of a JSON file a set.
//!
//! A DataSet's hash key, e.g. `<account>#<meter>#MeasurementsConsumption`, which has account and meter numbers in it,
//! becomes a store hash key of the kind `octopus.<name>`, its last part in lowercase, e.g.
//! `octopus.measurementsconsumption`, and the hash of the whole key, see the design's IDs, which keeps the numbers out
//! of the store's keys and file names. The key itself is kept in the DataSet's `#key` record, so that the files can
//! still be read and redacted, and when the DataSet was last fetched in its `#fetched` record, in place of a file's
//! modification time. A DataSet's file in the old layout is imported the first time it is opened, see import_legacy.

use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use bruce_db::{Flow, Id, Item, Micros, Part, Store};
use serde_json::Value;

use crate::CacheConfig;

const KEY: &str = "#key";
const FETCHED: &str = "#fetched";

/// The records of a DataSet, see the module documentation.
pub struct Records {
    store: Arc<Store>,
    hk: String,
}

/// The kind of a DataSet's hash key, e.g. octopus.measurementsconsumption, from its last part.
fn kind_of(hash_key: &str) -> anyhow::Result<String> {
    let name: String = hash_key
        .rsplit('#')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();
    let kind = format!("octopus.{name}");
    if name.is_empty() || !bruce_db::is_name(&kind) {
        bail!("{hash_key} has no name to make a kind of");
    }
    Ok(kind)
}

impl Records {
    /// The records of the DataSet hash_key, in the store of config, see the module documentation.
    pub fn open(config: &CacheConfig, hash_key: &str) -> anyhow::Result<Records> {
        let store = config.store()?.clone();
        let id = Id::of(&serde_json::json!({ "data-set": hash_key }))?;
        let hk = store.hash_key(&kind_of(hash_key)?, &[Part::Id(id)])?;
        let records = Records { store, hk };
        if records.get(KEY)?.is_none() {
            records.put(KEY, serde_json::json!({ "key": hash_key }))?;
        }
        Ok(records)
    }

    /// The records of the DataSet hash_key in store, to read, e.g. from a page which has the profile's store but not a
    /// module's CacheConfig, see open.
    pub fn in_store(store: Arc<Store>, hash_key: &str) -> anyhow::Result<Records> {
        let id = Id::of(&serde_json::json!({ "data-set": hash_key }))?;
        let hk = store.hash_key(&kind_of(hash_key)?, &[Part::Id(id)])?;
        Ok(Records { store, hk })
    }

    /// How long ago the DataSet was last fetched, None if it never has been, or if that was in the future.
    pub fn fetched_age(&self) -> anyhow::Result<Option<Duration>> {
        Ok(self.store.get(&self.hk, FETCHED)?.and_then(|item| {
            let age = Micros::now().0 - item.version.time.0;
            (age >= 0).then(|| Duration::from_micros(age as u64))
        }))
    }

    /// When the record sk was last put, from its version, None if there is none.
    pub fn fetched_at(&self, sk: &str) -> anyhow::Result<Option<time::OffsetDateTime>> {
        Ok(self
            .store
            .get(&self.hk, sk)?
            .and_then(|item| item.version.time.to_datetime().ok()))
    }

    /// Records that the DataSet has been fetched now.
    pub fn mark_fetched(&self) -> anyhow::Result<()> {
        self.put(FETCHED, serde_json::json!({}))
    }

    pub fn get(&self, sk: &str) -> anyhow::Result<Option<Value>> {
        Ok(self.store.get(&self.hk, sk)?.and_then(|item| item.attributes))
    }

    /// Puts a record, a new version written by the app, see store::version_now.
    pub fn put(&self, sk: &str, value: Value) -> anyhow::Result<()> {
        self.store
            .put(&Item::new(&self.hk, sk, crate::store::version_now(), value))?;
        Ok(())
    }

    /// Deletes a record, e.g. one the server no longer has.
    pub fn delete(&self, sk: &str) -> anyhow::Result<()> {
        self.store
            .put(&Item::tombstone(&self.hk, sk, crate::store::version_now()))?;
        Ok(())
    }

    /// Imports the DataSet's file in the old layout, relative to the module's cache directory, e.g. `#Viewer` or
    /// `2026/October#<hash key>`, the first time it is asked to, giving put each of its lines, so that a profile made
    /// before the store keeps its data, see convert.rs. Returns whether there was a file to import. A file is imported
    /// once, which its `#imported/<file>` record says, and is left as it was.
    pub fn import_legacy(
        &self,
        config: &CacheConfig,
        relative: &str,
        put: &mut dyn FnMut(&Records, Value) -> anyhow::Result<()>,
    ) -> anyhow::Result<bool> {
        let marker = format!("#imported/{relative}");
        let path = config.dir_path().join(relative);
        // An offline profile's are imported too, as it only shows what it has, which is in the store.
        if !path.exists() || self.get(&marker)?.is_some() {
            return Ok(false);
        }
        let text = std::fs::read_to_string(&path)?;
        let mut count = 0;
        // Synced once at the end rather than a record at a time, as a month can be thousands of records, and synced as
        // usual again however it ends; the marker is only put once they are all synced.
        self.store.set_sync(false);
        let imported = (|| -> anyhow::Result<()> {
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                match serde_json::from_str::<Value>(line) {
                    Ok(value) => {
                        put(self, value)?;
                        count += 1;
                    }
                    Err(error) => {
                        crate::log_warning!("Skipping a line of {} which is not JSON, {error}", path.display())
                    }
                }
            }
            Ok(())
        })();
        self.store.set_sync(true);
        self.store.sync()?;
        imported?;
        self.put(&marker, serde_json::json!({ "records": count }))?;
        crate::log_info!("Imported {count} records of {} into the store", path.display());
        Ok(true)
    }

    /// The records with sort keys in range, in order, leaving out the DataSet's own, whose keys start with #.
    pub fn range(&self, range: std::ops::Range<&str>) -> anyhow::Result<Vec<(String, Value)>> {
        let mut records = Vec::new();
        self.store.query(&self.hk, range, &mut |item| {
            if !item.sk.starts_with('#') {
                if let Some(value) = item.attributes {
                    records.push((item.sk, value));
                }
            }
            Ok(Flow::Continue)
        })?;
        Ok(records)
    }

    /// Every record, in order, leaving out the DataSet's own.
    pub fn all(&self) -> anyhow::Result<Vec<(String, Value)>> {
        self.range(bruce_db::ALL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_sets_kind_is_its_keys_last_part_and_its_key_is_kept_hashed() {
        assert_eq!(
            kind_of("A-1#123#MeasurementsConsumption").unwrap(),
            "octopus.measurementsconsumption"
        );
        assert_eq!(kind_of("A-1#Bills").unwrap(), "octopus.bills");
        assert!(kind_of("A-1#").is_err());

        let dir = std::env::temp_dir().join(format!("bruce-smart-home-records-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let site = Id::of(&serde_json::json!({"site": "records test"})).unwrap();
        let store = Arc::new(Store::open(&format!("file://{}", dir.display()), &site).unwrap());
        let config = CacheConfig::new(dir.join("cache"), false).with_store(store.clone());
        let records = Records::open(&config, "A-1#Bills").unwrap();
        assert!(!records.hk.contains("A-1"), "the account number is not in the hash key");
        assert_eq!(records.get(KEY).unwrap(), Some(serde_json::json!({"key": "A-1#Bills"})));
        assert_eq!(records.fetched_age().unwrap(), None);
        records.put("2026-10-01#1", serde_json::json!({"n": 1})).unwrap();
        records.mark_fetched().unwrap();
        assert!(
            records
                .fetched_age()
                .unwrap()
                .is_some_and(|age| age < Duration::from_secs(60))
        );
        assert_eq!(
            records.all().unwrap(),
            vec![("2026-10-01#1".to_string(), serde_json::json!({"n": 1}))]
        );
        records.delete("2026-10-01#1").unwrap();
        assert!(records.all().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_file_is_imported_once_and_left_as_it_was() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cache/2026")).unwrap();
        std::fs::write(
            dir.join("cache/2026/October#A-1#Bills"),
            "{\"n\": 1}\n\n{\"n\": 2}\nnot json\n",
        )
        .unwrap();
        let site = Id::of(&serde_json::json!({"site": "legacy test"})).unwrap();
        let store = Arc::new(Store::open(&format!("file://{}", dir.join("store").display()), &site).unwrap());
        let config = CacheConfig::new(dir.join("cache"), false).with_store(store.clone());
        let records = Records::open(&config, "A-1#Bills").unwrap();
        let import = |records: &Records| {
            let mut place = 0;
            records
                .import_legacy(&config, "2026/October#A-1#Bills", &mut |records, value| {
                    place += 1;
                    records.put(&format!("{place}"), value)
                })
                .unwrap()
        };
        assert!(import(&records));
        assert_eq!(
            records.all().unwrap().len(),
            2,
            "the line which is not JSON is left out"
        );
        assert!(!import(&records), "imported once");
        assert!(dir.join("cache/2026/October#A-1#Bills").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
