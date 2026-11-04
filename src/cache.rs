
use std::path::PathBuf;

use sparko_graphql::types::Date;
use time::Month;

pub struct CacheConfig {
    dir_path: PathBuf,
    pub verbose: bool,
    /// Only what is cached is used, nothing is fetched, for a redacted profile, see Profile::offline.
    pub offline: bool,
}

/// The configuration for a monthly bucket
pub struct MonthBucket {
    /// Path to the local cache file
    pub path: PathBuf,
    /// Start date for this bucket
    pub start: Date,
    /// Start date for the next bucket
    pub next: Date,
}

impl CacheConfig {
    /// The module's cache directory, which holds all its cache files.
    pub fn dir_path(&self) -> &std::path::Path {
        &self.dir_path
    }

    /// The cache files whose hash key starts with key, e.g. snapshot- for 2026/October/28#snapshot-134054, newest
    /// first by when they were written.
    pub fn files_with_key(&self, key: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut dirs = vec![self.dir_path.clone()];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                }
                else if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.contains(&format!("#{key}"))) {
                    if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) {
                        found.push((modified, path));
                    }
                }
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0));
        found.into_iter().map(|(_, path)| path).collect()
    }

    /// The contents of a JSON cache file, None if it cannot be read.
    pub fn read_json(path: &std::path::Path) -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    /// The contents of the latest cache file whose hash key starts with key, see files_with_key.
    pub fn latest_json(&self, key: &str) -> Option<serde_json::Value> {
        Self::read_json(self.files_with_key(key).first()?)
    }

    pub fn path_for_hash_key(&self, hash_key: &str) -> PathBuf {
        let mut path = self.dir_path.clone();
        path.push(hash_key);
        path
    }

    pub fn path_for_month(&self, hash_key: &str, date: &Date) -> anyhow::Result<MonthBucket> {
        let start = Date::from_calendar_date(date.year(), date.month(), 1)?;
        let next = if date.month() == Month::December {
            Date::from_calendar_date(date.year() + 1, Month::January, 1)?
        }
        else {
            Date::from_calendar_date(date.year(), date.month().next(), 1)?
        };

        let mut path = self.dir_path.clone();

        Self::path_for_date(&mut path, date);
        // The year directory does not exist the first time a month in that year is cached.
        crate::private_file::create_private_dir(&path)?;
        Self::path_hash_key_for_date(&mut path, date, hash_key);
        Ok( MonthBucket {path, start, next})
    }

/// The cache file for the day date, in a directory for its month in the directory for its year, e.g.
    /// 2026/October/25#hash_key
    /// The path of the cache file for the day date, see path_for_day, without making its directories, to read it.
    pub fn existing_path_for_day(&self, hash_key: &str, date: &Date) -> PathBuf {
        let mut path = self.dir_path.clone();
        Self::path_for_date(&mut path, date);
        path.push(date.month().to_string());
        path.push(format!("{}#{}", date.day(), hash_key));
        path
    }

    pub fn path_for_day(&self, hash_key: &str, date: &Date) -> anyhow::Result<PathBuf> {
        let mut path = self.dir_path.clone();
        Self::path_for_date(&mut path, date);
        // The directories do not exist the first time a day in that year or month is cached.
        crate::private_file::create_private_dir(&path)?;
        path.push(date.month().to_string());
        crate::private_file::create_private_dir(&path)?;
        path.push(format!("{}#{}", date.day(), hash_key));
        Ok(path)
    }

    fn path_for_date(path: &mut PathBuf, date: &Date) {
        path.push(date.year().to_string());
        // path.push(date.month().to_string());
    }

    fn path_hash_key_for_date(path: &mut PathBuf, date: &Date, hash_key: &str) {
        // path.push(format!("{}#{}", date.day(), hash_key));
        path.push(format!("{}#{}", date.month(), hash_key));
    }
    
    pub fn new(dir_path: PathBuf, verbose: bool, offline: bool) -> Self {
        CacheConfig {
            dir_path,
            verbose,
            offline,
        }
    }
}