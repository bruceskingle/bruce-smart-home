use std::path::PathBuf;

/// A module's configuration for keeping its data: the profile's store, see store.rs, which it keeps it in, whether the
/// profile is offline, and the module's directory in the cache directory, which the Cache Files pages still show.
#[derive(Clone)]
pub struct CacheConfig {
    dir_path: PathBuf,
    /// Only what is cached is used, nothing is fetched, for a redacted profile, see Profile::offline.
    pub offline: bool,
    /// The profile's store, None in tests which do not need one.
    store: Option<std::sync::Arc<bruce_db::Store>>,
}

impl CacheConfig {
    /// The module's cache directory.
    pub fn dir_path(&self) -> &std::path::Path {
        &self.dir_path
    }

    pub fn new(dir_path: PathBuf, offline: bool) -> Self {
        CacheConfig {
            dir_path,
            offline,
            store: None,
        }
    }

    /// The same, with the profile's store, see store.
    pub fn with_store(mut self, store: std::sync::Arc<bruce_db::Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// The same, reading only what is stored, nothing fetched, as an offline profile does, e.g. for a page to draw at
    /// once from what the profile has, before it fetches what is missing, see ElectricitySource::stored.
    pub fn stored_only(&self) -> Self {
        CacheConfig {
            dir_path: self.dir_path.clone(),
            offline: true,
            store: self.store.clone(),
        }
    }

    /// The profile's store.
    pub fn store(&self) -> anyhow::Result<&std::sync::Arc<bruce_db::Store>> {
        self.store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("The cache has no store"))
    }
}
