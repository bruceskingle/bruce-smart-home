//! The store, bruce-db, which the modules keep their data in, see docs/Store.md and bruce-db's docs/design.md: a
//! directory of sites in the data home, one site for each profile, see Profile::site.
//!
//! What a source gave is kept as it gave it, and anything worked out from it, if it is kept at all, in kinds of its
//! own, see AGENTS.md's "Caches hold only what the source sent". Versions written by the app have this installation's
//! node as their origin, see origin.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bruce_db::{Id, Micros, Store, Version};

/// The directory of every profile's site's store, in the data directory, see data_home.rs.
pub fn store_dir() -> anyhow::Result<PathBuf> {
    crate::data_home::store_dir()
}

/// The store of the profile's site, made if it has none, see profile::ensure_site.
pub fn open(profile_name: &str) -> anyhow::Result<Store> {
    let site = crate::profile::ensure_site(profile_name)?;
    let dir = store_dir()?;
    crate::private_file::create_private_dir(&dir)?;
    Store::open(&format!("file://{}", dir.display()), &site)
}

/// The store of the profile's site, see open, opened once and shared, as a store keeps its journal in memory, see
/// bruce-db's file.rs, and pages open it by the profile's name each time they are drawn.
pub fn for_profile(profile_name: &str) -> anyhow::Result<Arc<Store>> {
    static STORES: OnceLock<Mutex<HashMap<String, Arc<Store>>>> = OnceLock::new();
    let mut stores = STORES.get_or_init(Default::default).lock().unwrap();
    if let Some(store) = stores.get(profile_name) {
        return Ok(store.clone());
    }
    let store = Arc::new(open(profile_name)?);
    stores.insert(profile_name.to_string(), store.clone());
    drop(stores);
    compact_in_background(profile_name, store.clone());
    Ok(store)
}

/// How long after a store is opened its journal is first compacted, so that the first page is drawn before it: a
/// compaction holds the store until it is done, see bruce-db's file.rs, and pages wait for it.
const FIRST_COMPACTION: Duration = Duration::from_secs(60);

/// How often the journal is compacted after that, so that an app left running for days keeps it small. When there is
/// nothing to compact, it only lists the journal's files.
const COMPACTION_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Compacts the store's journal on a thread of its own, see FIRST_COMPACTION and COMPACTION_INTERVAL: its days before
/// yesterday are folded into the DataSets' files, leaving a day for late records, see bruce-db's design, so that the
/// journal, which is kept in memory and read whenever the store is opened, stays small. How long it takes is in the
/// profile's log, from bruce-db.
fn compact_in_background(profile_name: &str, store: Arc<Store>) {
    let spawned = std::thread::Builder::new()
        .name(format!("compact {profile_name}"))
        .spawn(move || {
            std::thread::sleep(FIRST_COMPACTION);
            loop {
                if let Some(before) = Micros::now().date().ok().and_then(|today| today.previous_day())
                    && let Err(error) = store.compact(before)
                {
                    crate::log_warning!("Unable to compact the store, {error:#}");
                }
                std::thread::sleep(COMPACTION_INTERVAL);
            }
        });
    if let Err(error) = spawned {
        crate::log_warning!("Unable to start compacting the store, {error:#}");
    }
}

/// A part of a hash key for a capture device, its ID, a MAC address, in lowercase without its colons, e.g.
/// 441bf6cc7fcc, which is also its origin, see docs/Store.md's kinds.
pub fn device_part(device_id: &str) -> String {
    crate::device_kinds::device_part(device_id)
}

/// The directory of a site's store, e.g. to count or delete it with its profile.
pub fn site_dir(site: &Id) -> anyhow::Result<PathBuf> {
    Ok(store_dir()?.join(site.hex()))
}

/// A new site for a profile, its ID made, as a created thing's is, see the design's IDs, from what made it, this
/// installation, when, and the profile's name then, which may later change.
pub fn new_site(profile_name: &str) -> anyhow::Result<Id> {
    Id::of(&serde_json::json!({
        "site": {"profile": profile_name, "created": Micros::now().to_text(), "node": node()?},
    }))
}

/// This installation's node, 12 hex characters made at random once and kept in the settings, as a device's is its MAC
/// address, see the design's IDs.
pub fn node() -> anyhow::Result<String> {
    let path = crate::data_home::settings_dir()?.join("node");
    if let Ok(node) = std::fs::read_to_string(&path) {
        let node = node.trim().to_string();
        if node.len() == 12 && node.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(node);
        }
    }
    let node: String = (0..6).map(|_| format!("{:02x}", rand::random::<u8>())).collect();
    std::fs::write(&path, &node)?;
    Ok(node)
}

/// The origin of what the app writes, see Version, which sorts after any device's ID of digits, so that of two equal
/// versions a device's wins, as it is the source's own.
pub fn origin() -> String {
    // A test does not read or make the real data home's node.
    if cfg!(test) {
        return String::from("app-000000000000");
    }
    format!("app-{}", node().unwrap_or_else(|_| String::from("000000000000")))
}

/// A version of now, written by the app.
pub fn version_now() -> Version {
    Version::new(Micros::now(), &origin())
}
