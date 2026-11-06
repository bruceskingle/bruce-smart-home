//! The device's store, bruce-db, in /sd/store/<site hex>/, which each feature puts its records in, as the items the app
//! would put, see marco-sparko's src/device_kinds.rs, and which the app reads through the store's change feed,
//! /api/store/changes, see bruce-db's docs/design.md, "Capture devices on the store".
//!
//! The site is a setting, which the app sets when it adopts the device; until it is set, or if there is no SD card,
//! there is no store, and the records are only shown on the screen. The records kept before the store, in
//! /sd/capture/<feature>/, are converted into it once, see convert, and left where they are.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bruce_db::{Cursor, Id, Item, Micros, Part, Store};
use chrono::{DateTime, Utc};
use log::{info, warn};
use serde_json::{Value, json};
use sparko_embedded_std::config::ConfigSpec;
use sparko_embedded_std::config::ConfigSpecValue;
use sparko_embedded_std::config::FeatureConfig;
use sparko_embedded_std::config::TypedValue;
use sparko_embedded_std::feature::FeatureDescriptor;
use sparko_embedded_std::platform::PlatformInitializer;
use sparko_embedded_std::task::scheduler::ScheduledTask;
use sparko_esp_idf::{Esp32Platform, Esp32PlatformInitializer, Feature};

use crate::device_kinds::{self, Writer};
use crate::screen::Panel;

/// The feature's name, which is also its NVS namespace, so at most 15 characters.
const FEATURE_NAME: &str = "Store";

/// Records of which features' old records have been converted, a DataSet for the device, by the feature's name, e.g.
/// tesla, see convert.
const CONVERTED_KIND: &str = "device.converted";

/// Where the conversion's progress goes on the screen, after the features' lines.
const PANEL_ORDER: u8 = 9;

/// The most changes given at once, see changes, and how many if the app does not say.
const MAX_CHANGES: usize = 1000;
const DEFAULT_CHANGES: usize = 500;

/// When the journal's finished days are compacted, at night, when little is polled, see CompactTask.
const COMPACT_SCHEDULE: &str = "0 7 3 * * *";

#[derive(FeatureConfig)]
#[config(label = "Store")]
pub struct StoreConfig {
    /// The ID of the site of the profile which adopted the device, which the app sets.
    #[config(len = 64, label = "Store Site (set by the app)")]
    store_site: Option<String>,
}

/// A feature's status for the app, e.g. how its latest poll went, see Device::to_json, given the store.
type Status = Box<dyn Fn(&Db) -> Value + Send + Sync>;

/// The device's store, if it has one, shared by the features, see the module documentation.
pub struct Db {
    state: Mutex<State>,
    statuses: Mutex<Vec<(&'static str, Status)>>,
}

#[derive(Default)]
struct State {
    /// The device's ID, the origin of what it writes.
    device: String,
    /// Where the SD card is mounted, if it is.
    mount_point: Option<PathBuf>,
    store: Option<Arc<Store>>,
    /// Why there is no store, or why the last put failed.
    problem: Option<String>,
}

impl Db {
    pub fn new() -> Arc<Db> {
        Arc::new(Db { state: Mutex::new(State::default()), statuses: Mutex::new(Vec::new()) })
    }

    pub fn store(&self) -> Option<Arc<Store>> {
        self.state.lock().unwrap().store.clone()
    }

    /// The device's ID, see State.
    pub fn device(&self) -> String {
        self.state.lock().unwrap().device.clone()
    }

    /// Why there is no store, or why the last put failed, for the screen and the app.
    pub fn problem(&self) -> Option<String> {
        self.state.lock().unwrap().problem.clone()
    }

    /// Puts the item make makes of a record, written by this device, if there is a store, and make makes one,
    /// noting the problem if that fails, and returning whether it was stored.
    pub fn put(&self, make: impl FnOnce(&Store, Writer) -> anyhow::Result<Option<Item>>) -> bool {
        let (store, device) = {
            let state = self.state.lock().unwrap();
            (state.store.clone(), state.device.clone())
        };
        let Some(store) = store else { return false };
        let result = make(&store, Writer::Device(&device)).and_then(|item| match item {
            Some(item) => Ok(store.put(&item)? == bruce_db::Put::Stored),
            None => Ok(false),
        });
        let mut state = self.state.lock().unwrap();
        match result {
            Ok(stored) => {
                state.problem = None;
                stored
            }
            Err(error) => {
                warn!("Unable to put a record in the store, {error:#}");
                state.problem = Some(format!("{error:#}"));
                false
            }
        }
    }

    /// Adds a feature's status, which /api/store/info gives, see info.
    pub fn add_status(&self, feature: &'static str, status: impl Fn(&Db) -> Value + Send + Sync + 'static) {
        self.statuses.lock().unwrap().push((feature, Box::new(status)));
    }

    /// The SD card's size and free space, if the store is on one.
    pub fn sd_card(&self) -> Option<Value> {
        self.store()?;
        sparko_esp_idf::sd_card::space().map(|(total, free)| json!({ "totalBytes": total, "freeBytes": free }))
    }

    /// Opens the site's store on the card mounted at mount_point, saying why not if it can not.
    fn open(&self, device: String, mount_point: Result<&str, String>, site: Option<&str>) {
        let mut state = self.state.lock().unwrap();
        state.device = device;
        let mount_point = match mount_point {
            Ok(mount_point) => PathBuf::from(mount_point),
            Err(problem) => {
                state.problem = Some(format!("No SD card, {problem}"));
                return;
            }
        };
        state.mount_point = Some(mount_point.clone());
        let Some(site) = site.map(str::trim).filter(|site| !site.is_empty()) else {
            state.problem = Some("No site: the app sets it when it adopts the device".to_string());
            return;
        };
        let opened = Id::parse(site).or_else(|_| Id::from_hex(site)).and_then(|site| {
            Store::open(&format!("file://{}", mount_point.join("store").display()), &site)
        });
        match opened {
            Ok(store) => {
                info!("Store {} opened for the site {site}", store.store_id());
                state.store = Some(Arc::new(store));
                state.problem = None;
            }
            Err(error) => {
                warn!("Unable to open the store, {error:#}");
                state.problem = Some(format!("Unable to open the store, {error:#}"));
            }
        }
    }

    /// Converts the records a feature kept before the store, in /sd/capture/<feature>/, each line made an item by
    /// make, once: a record of CONVERTED_KIND says it has been, so that a conversion cut short, e.g. by a restart, is
    /// done again, which puts again what was put, changing nothing, see the design. Its progress is on the screen.
    pub fn convert(&self, panel: &Panel, feature: &str, make: &dyn Fn(&Store, Writer, &Value) -> anyhow::Result<Option<Item>>) {
        if let Err(error) = self.try_convert(panel, feature, make) {
            warn!("Unable to convert the old {feature} records, {error:#}");
            panel.set(PANEL_ORDER, vec![(format!("Converting {feature}: {error:#}"), true)]);
        }
    }

    fn try_convert(&self, panel: &Panel, feature: &str, make: &dyn Fn(&Store, Writer, &Value) -> anyhow::Result<Option<Item>>)
        -> anyhow::Result<()> {
        let (store, device, mount_point) = {
            let state = self.state.lock().unwrap();
            (state.store.clone(), state.device.clone(), state.mount_point.clone())
        };
        let (Some(store), Some(mount_point)) = (store, mount_point) else { return Ok(()) };
        let hk = store.hash_key(CONVERTED_KIND, &[Part::Name(device_kinds::device_part(&device))])?;
        if store.get(&hk, feature)?.is_some() {
            return Ok(());
        }
        let dir = mount_point.join("capture").join(feature);
        let (mut read, mut stored, mut skipped) = (0usize, 0usize, 0usize);
        if dir.exists() {
            info!("Converting the old {feature} records in {}", dir.display());
            panel.set(PANEL_ORDER, vec![(format!("Converting old {feature} records"), false)]);
            let writer = Writer::Device(&device);
            crate::day_files::write_since(&dir, Some(DateTime::<Utc>::UNIX_EPOCH), None, &mut |line| {
                read += 1;
                let item = serde_json::from_slice::<Value>(line).map_err(anyhow::Error::from)
                    .and_then(|record| make(&store, writer, &record));
                match item {
                    Ok(Some(item)) => {
                        if store.put(&item)? == bruce_db::Put::Stored {
                            stored += 1;
                        }
                    }
                    // e.g. a line cut short by a power cut, or another feature's on a damaged card.
                    Ok(None) | Err(_) => skipped += 1,
                }
                if read % 200 == 0 {
                    panel.set(PANEL_ORDER, vec![(format!("Converting {feature}: {read} records"), false)]);
                }
                Ok(())
            })?;
        }
        store.put(&Item::new(&hk, feature, bruce_db::Version::new(Micros::now(), &device_kinds::device_part(&device)),
            json!({ "read": read, "stored": stored, "skipped": skipped, "converted": Micros::now().to_text() })))?;
        info!("Converted the old {feature} records: {read} read, {stored} stored, {skipped} skipped");
        panel.set(PANEL_ORDER, Vec::new());
        Ok(())
    }

    /// The store's details and each feature's status, for /api/store/info.
    fn info(&self) -> Value {
        let store = self.store();
        let statuses = self.statuses.lock().unwrap();
        let features: serde_json::Map<String, Value> =
            statuses.iter().map(|(feature, status)| (feature.to_string(), status(self))).collect();
        json!({
            "device": self.device(),
            "firmware": crate::device::FIRMWARE,
            "site": store.as_ref().map(|store| store.site().to_string()),
            "store": store.as_ref().map(|store| store.store_id()),
            "format": 1,
            "problem": self.problem(),
            "sdCard": self.sd_card(),
            "features": features,
        })
    }
}

/// The store's setting, and its pages on the API.
pub struct StoreFeature {
    db: Arc<Db>,
    panel: Arc<Panel>,
}

impl StoreFeature {
    pub fn new(db: Arc<Db>, panel: Arc<Panel>) -> Self {
        Self { db, panel }
    }
}

impl Feature for StoreFeature {
    type Config = StoreConfig;

    fn init(&self, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<FeatureDescriptor> {
        self.panel.start(initializer)?;
        Ok(FeatureDescriptor { name: FEATURE_NAME.to_string(), config: StoreConfig::to_config_spec()? })
    }

    fn start(
        &mut self,
        sparko: &mut Esp32Platform,
        initializer: &mut Esp32PlatformInitializer,
        config: StoreConfig,
    ) -> anyhow::Result<()> {
        let device = sparko.device_id().unwrap_or_default();
        self.db.open(device.clone(), sparko.sd_card(), config.store_site.as_deref());
        let name = crate::device::captured_by(sparko);
        self.db.put(|store, writer| Ok(Some(device_kinds::device_info(store, &device, &name, Micros::now(), writer)?)));

        // The old records of the features which do not need anything from a poll to know their records' keys, in a
        // thread of their own, while the features poll; Octopus's needs its account, see octopus.rs.
        let db = self.db.clone();
        let panel = self.panel.clone();
        std::thread::Builder::new().stack_size(16 * 1024).spawn(move || {
            db.convert(&panel, "tesla", &|store, writer, record| Ok(Some(device_kinds::tesla_snapshot(store, record, writer)?)));
            db.convert(&panel, "myenergi", &|store, writer, record| {
                Ok(Some(device_kinds::myenergi_status(store, record, writer)?))
            });
            let device = db.device();
            db.convert(&panel, "climate", &|store, _, record| device_kinds::climate_period(store, &device, record));
        })?;

        initializer.add_task(Box::new(CompactTask { db: self.db.clone(), running: Arc::new(AtomicBool::new(false)) }), COMPACT_SCHEDULE)?;

        let db = self.db.clone();
        sparko.api_get("/api/store/info", move |request| {
            sparko_esp_idf::api::respond(request, 200, &db.info())
        })?;
        let db = self.db.clone();
        sparko.api_get("/api/store/changes", move |request| changes(request, &db))?;
        Ok(())
    }
}

/// Compacts the journal's days before yesterday, see bruce-db's design, in a thread of its own, as it takes a while,
/// and the scheduler runs the polls.
struct CompactTask {
    db: Arc<Db>,
    running: Arc<AtomicBool>,
}

impl ScheduledTask<Esp32Platform> for CompactTask {
    fn run(&mut self, _sparko: &mut Esp32Platform) -> anyhow::Result<()> {
        let Some(store) = self.db.store() else { return Ok(()) };
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let running = self.running.clone();
        std::thread::Builder::new().stack_size(16 * 1024).spawn(move || {
            if let Some(before) = Micros::now().date().ok().and_then(|today| today.previous_day()) {
                info!("Compacting the store's journal before {before}");
                match store.compact(before) {
                    Ok(()) => info!("Compacted the store"),
                    Err(error) => warn!("Unable to compact the store, {error:#}"),
                }
            }
            running.store(false, Ordering::SeqCst);
        })?;
        Ok(())
    }

    fn name(&self) -> &str {
        "Store Compaction"
    }
}

/// Responds to a request for the store's changes after the sequence number in its since parameter, 0 if it has none,
/// up to its limit parameter, as {"device", "site", "store", "changes": [...], "next": n}, each change a journal's line,
/// see bruce_db::Change::to_json, and next the cursor to ask from next time, which the app asks from until it stops
/// moving. Written a change at a time, as a page of them would not all fit in the RAM as one piece of JSON. If reading
/// them fails, the error is at the end, as "error", after the changes before it, which the app can still put.
fn changes(
    request: esp_idf_svc::http::server::Request<&mut esp_idf_svc::http::server::EspHttpConnection<'_>>,
    db: &Db,
) -> anyhow::Result<()> {
    let since = parameter(request.uri(), "since").and_then(|value| value.parse().ok()).unwrap_or(0);
    let limit = parameter(request.uri(), "limit")
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_CHANGES)
        .min(MAX_CHANGES);
    let Some(store) = db.store() else {
        let problem = db.problem().unwrap_or_else(|| "There is no store".to_string());
        return sparko_esp_idf::api::respond(request, 503, &json!({ "error": problem }));
    };
    let response = request.into_response(200, None, &[("Content-Type", "application/json")])?;
    let mut out = Chunks { out: response, buffer: Vec::with_capacity(CHUNK + 512) };
    out.write(br#"{"device":"#)?;
    out.write(&serde_json::to_vec(&db.device())?)?;
    out.write(br#","site":"#)?;
    out.write(&serde_json::to_vec(&store.site().to_string())?)?;
    out.write(br#","store":"#)?;
    out.write(&serde_json::to_vec(&store.store_id())?)?;
    out.write(br#","changes":["#)?;
    // The changes are read in a thread of their own, as reading needs more stack than the HTTP server's task has, and
    // handed over a few at a time, so that the response is still sent as they are read. The reader stops when the
    // response is given up, as the receiver is then dropped.
    let (sender, receiver) = std::sync::mpsc::sync_channel::<Vec<u8>>(CHANGES_QUEUE);
    let store = &store;
    let result = std::thread::scope(|scope| -> anyhow::Result<anyhow::Result<Cursor>> {
        let reader = std::thread::Builder::new().stack_size(16 * 1024).spawn_scoped(scope, move || {
            store.changes(Cursor(since), limit, None, &mut |change| {
                sender.send(serde_json::to_vec(&change.to_json()?)?)
                    .map_err(|_| anyhow::Error::new(SendFailed("the response was given up".to_string())))
            })
        });
        // The response has begun, so a reader which could not be started is given as the response's error.
        let reader = match reader {
            Ok(reader) => reader,
            Err(error) => return Ok(Err(anyhow::Error::new(error).context("starting to read the changes"))),
        };
        let mut first = true;
        let mut written = Ok(());
        for bytes in receiver {
            written = if first { Ok(()) } else { out.write(b",") }.and_then(|_| out.write(&bytes));
            first = false;
            if written.is_err() {
                break;
            }
        }
        let read = reader.join().map_err(|_| anyhow::anyhow!("reading the changes panicked"))?;
        written?;
        Ok(read)
    })?;
    let next = match result {
        Ok(next) => next.0,
        Err(error) if error.is::<SendFailed>() => {
            warn!("Unable to send the changes, {error:#}");
            return Err(error);
        }
        Err(error) => {
            warn!("Unable to read the changes, {error:#}");
            out.write(br#"],"error":"#)?;
            out.write(&serde_json::to_vec(&format!("{error:#}"))?)?;
            out.write(b"}")?;
            return out.flush();
        }
    };
    out.write(br#"],"next":"#)?;
    out.write(next.to_string().as_bytes())?;
    out.write(b"}")?;
    out.flush()
}

/// A parameter of a request's query string, e.g. since of ?since=1200.
fn parameter(uri: &str, name: &str) -> Option<String> {
    let (_, query) = uri.split_once('?')?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(parameter, _)| parameter == name)
        .map(|(_, value)| value.into_owned())
}

/// How much of a response is sent at a time, see Chunks, rather than a chunk for each change and each comma between
/// them, which is slow, as each is sent on its own.
const CHUNK: usize = 1536;
/// How many changes the reader may get ahead of the response by, see changes.
const CHANGES_QUEUE: usize = 16;

/// Sending the response failed, e.g. the app went away, after which nothing more can be sent on the connection, as
/// the HTTP server may have sent only part of a chunk, so the response is given up, rather than taken for the card
/// failing.
#[derive(Debug)]
struct SendFailed(String);

impl std::fmt::Display for SendFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sending the response, {}", self.0)
    }
}

impl std::error::Error for SendFailed {}

/// A response sent in pieces of about CHUNK bytes.
struct Chunks<W> {
    out: W,
    buffer: Vec<u8>,
}

impl<W: esp_idf_svc::io::Write> Chunks<W> {
    fn write(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= CHUNK {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let result = self.out.write_all(&self.buffer);
        self.buffer.clear();
        result.map_err(|error| SendFailed(format!("{error:?}")).into())
    }
}
