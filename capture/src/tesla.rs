//! Polls the local API of the Tesla Powerwall's gateway, as bruce-smart-home's src/tesla/local.rs does, which says more.
//!
//! Other programs, e.g. Home Assistant, use the gateway too, so this must never get in their way:
//! - Only GETs of the endpoints in records::ENDPOINTS, which change nothing.
//! - It logs in as the customer and reuses the token it is given, logging in at most once a poll.
//! - A rejected password is removed from the config, so it is not tried again, even after a restart, as repeated
//!   failures lock out the customer login, for every program.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use croner::Cron;
use embedded_svc::http::client::Client;
use embedded_svc::io::Write;
use esp_idf_svc::http::Method;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use log::{info, warn};
use serde::Deserialize;
use serde_json::json;
use bruce_embedded_std::config::ConfigSpec;
use bruce_embedded_std::config::ConfigSpecValue;
use bruce_embedded_std::config::FeatureConfig;
use bruce_embedded_std::config::TypedValue;
use bruce_embedded_std::feature::FeatureDescriptor;
use bruce_embedded_std::platform::PlatformInitializer;
use bruce_embedded_std::task::scheduler::ScheduledTask;
use bruce_esp_idf::{Esp32Platform, Esp32PlatformInitializer, Feature};

use crate::connections::KeptConnection;
use crate::graph::Sample;
use crate::device::{Device, captured_by};
use crate::slots::Slots;
use crate::db::Db;
use crate::records::{ENDPOINTS, Record, Response, Snapshots, format_time};
use crate::screen::{Line, Panel};

/// The feature's name, which is also its NVS namespace, so at most 15 characters.
const FEATURE_NAME: &str = "TeslaCapture";

/// The name of the password in the config, see forget_password.
const PASSWORD: &str = "tesla_password";

/// Every 10 seconds, on the 5 minute slots the app polls on too, see bruce-smart-home's src/polling.rs, which the gateway, on
/// the home network, can easily answer, as the connection to it is kept between polls, see PollTask::fetch. The power
/// at that resolution is what the Octopus Home Mini's telemetry has, for comparison.
const DEFAULT_SCHEDULE: &str = "*/10 * * * * *";

/// How long to wait before logging in again after the gateway said there had been too many attempts.
const LOCKOUT_WAIT: TimeDelta = TimeDelta::minutes(30);

/// The largest response read, to protect the RAM. meters/aggregates, the largest polled, is about 2.5 KB.
const MAX_BODY: usize = 16 * 1024;

/// The config's names must differ from every other feature's, as the config page's form has one field for each name,
/// and at most 15 characters, as they are NVS keys.
#[derive(FeatureConfig)]
#[config(label = "Tesla Powerwall")]
pub struct TeslaConfig {
    /// The gateway's address on the home network.
    #[config(len = 64, label = "Tesla Gateway Hostname or IP Address")]
    tesla_host: String,
    /// The email of the gateway's customer login.
    #[config(len = 64, label = "Tesla Gateway Customer Email")]
    tesla_email: String,
    /// Optional, so that the config is still valid when a rejected one is removed.
    #[config(len = 64, secret, label = "Tesla Gateway Customer Password")]
    tesla_password: Option<String>,
    #[config(label = "Tesla Poll Schedule (cron, seconds first; every 10 seconds if empty)")]
    tesla_schedule: Option<Cron>,
}

/// Where its lines go on the screen, first.
/// Where the meters' lines go, which a screen with the graph leaves out, see screen.rs, and the rest of the lines.
const PANEL_ORDER: u8 = crate::screen::METERS_SECTION + 1;

/// The meters, as named by the gateway, and as shown on the screen.
const METERS: &[(&str, &str)] = &[("site", "Grid"), ("solar", "Solar"), ("battery", "Battery"), ("load", "House")];

pub struct TeslaCapture {
    panel: Arc<Panel>,
    db: Arc<Db>,
    /// The connection to the gateway, kept between polls, see PollTask::fetch.
    gateway: KeptConnection,
}

impl TeslaCapture {
    pub fn new(panel: Arc<Panel>, db: Arc<Db>, gateway: KeptConnection) -> Self {
        Self { panel, db, gateway }
    }
}

impl Feature for TeslaCapture {
    type Config = TeslaConfig;

    fn init(&self, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<FeatureDescriptor> {
        self.panel.start(initializer)?;
        Ok(FeatureDescriptor {
            name: FEATURE_NAME.to_string(),
            config: TeslaConfig::to_config_spec()?,
        })
    }

    fn start(
        &mut self,
        sparko: &mut Esp32Platform,
        initializer: &mut Esp32PlatformInitializer,
        config: TeslaConfig,
    ) -> anyhow::Result<()> {
        let schedule_spec = config
            .tesla_schedule
            .map(|schedule| schedule.pattern.to_string())
            .unwrap_or_else(|| DEFAULT_SCHEDULE.to_string());
        let store = Arc::new(Mutex::new(Snapshots::new(self.db.clone(), captured_by(sparko))));
        let device = Arc::new(Device::new(&schedule_spec));

        let mut poll = PollTask {
            gateway: Gateway {
                host: config.tesla_host,
                email: config.tesla_email,
                password: config.tesla_password,
                token: None,
                no_login_until: None,
            },
            store: store.clone(),
            panel: self.panel.clone(),
            slots: Slots::new(&schedule_spec)?,
            client: self.gateway.clone(),
        };

        // The graph's history from the store, as the samples in RAM went with the restart.
        fill_graph(&self.panel, &self.db);
        set_panel(&self.panel, &store.lock().unwrap());

        // A first poll now, as the app does, so there is something to show without waiting for the first slot.
        poll.poll(sparko, None);
        initializer.add_task(Box::new(poll), &schedule_spec)?;

        // For the app, at /api/store/info, see db.rs.
        self.db.add_status("tesla", move |db| {
            let store = store.lock().unwrap();
            device.to_json(db, store.last_poll())
        });

        Ok(())
    }
}

/// Adds the samples of the store's snapshots from the last graph::HISTORY to the graph, oldest first, so that it shows
/// them at once after a restart. A record which can not be read is left out, as is the whole history if the store can
/// not be.
fn fill_graph(panel: &Panel, db: &Db) {
    let Some(store) = db.store() else { return };
    let from = crate::device_kinds::second_key(bruce_db::Micros((Utc::now() - crate::graph::HISTORY).timestamp_micros()));
    let mut added = 0;
    let result = crate::device_kinds::tesla_snapshot_key(&store).and_then(|hk| {
        store.query(&hk, from.as_str()..bruce_db::ALL.end, &mut |item| {
            let Some(json) = item.attributes else { return Ok(bruce_db::Flow::Continue) };
            let time = crate::device_kinds::tesla_time(&json).and_then(|time| DateTime::from_timestamp_micros(time.0));
            let meters = &json["responses"]["meters/aggregates"];
            let power = crate::graph::SERIES.map(|(name, _)| meters[name]["instant_power"].as_f64().map(|power| power as f32));
            if let (Some(time), [Some(grid), Some(solar), Some(battery), Some(house)]) = (time, power) {
                panel.add_sample(Sample { time, power: [grid, solar, battery, house] });
                added += 1;
            }
            Ok(bruce_db::Flow::Continue)
        })
    });
    match result {
        Ok(()) => info!("Filled the graph with {added} samples from the store"),
        Err(error) => warn!("Unable to fill the graph from the store, {error:#}"),
    }
}

/// What the screen shows, see panel_lines, and the latest sample for its graph, see graph.rs.
fn set_panel(panel: &Panel, store: &Snapshots) {
    let mut meters = Vec::new();
    if let Some(record) = store.latest() {
        for (name, label) in METERS {
            if let Some(meter) = record.meter(name) {
                meters.push((format!("{label:<8}{:>7.2} kW", meter.instant_power / 1000.0), false));
            }
        }
        let power = crate::graph::SERIES.map(|(name, _)| record.meter(name).map(|meter| meter.instant_power));
        if let [Some(grid), Some(solar), Some(battery), Some(house)] = power {
            panel.add_sample(Sample { time: record.time(), power: [grid, solar, battery, house] });
        }
    }
    panel.set(crate::screen::METERS_SECTION, meters);
    panel.set(PANEL_ORDER, panel_lines(store));
}

/// What the screen shows besides the power of each of the gateway's meters: the battery's charge and backup reserve,
/// and when the gateway was last polled, or why that failed.
fn panel_lines(store: &Snapshots) -> Vec<Line> {
    let mut lines = Vec::new();
    if let Some(record) = store.latest() {
        // As the gateway says them, which the Tesla app shows less the 5% it keeps in reserve.
        let charge = match record.response("system_status/soe") {
            Some(Response::Soe { percentage: Some(percentage) }) => format!("{percentage:.0}%"),
            _ => "?".to_string(),
        };
        let reserve = match record.response("operation") {
            Some(Response::Operation { backup_reserve_percent: Some(reserve), .. }) => format!("{reserve:.0}%"),
            _ => "?".to_string(),
        };
        lines.push((format!("Charge {charge}  min {reserve}"), false));
    }
    match store.last_poll() {
        None => lines.push(("Waiting for first poll".to_string(), false)),
        Some(poll) => {
            let time = poll.time.with_timezone(&chrono::Local).format("%H:%M");
            match (&poll.error, store.db().problem()) {
                (None, None) => lines.push((format!("Polled {time}"), false)),
                // e.g. no SD card or no site, so the records are not kept.
                (None, Some(_)) => lines.push((format!("Polled {time}, not stored"), true)),
                (Some(error), _) => lines.push((format!("{time} {error}"), true)),
            }
        }
    }
    lines
}

/// The gateway did not accept the email and password, which must not be tried again until they are changed.
#[derive(Debug)]
struct PasswordRejected;

impl std::fmt::Display for PasswordRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the gateway did not accept the email and password, which have been removed, set them again")
    }
}

impl std::error::Error for PasswordRejected {}

/// The gateway responded with a status other than success, after which the poll goes on to the next endpoint, unlike
/// a failure to reach it, which stops the poll.
#[derive(Debug)]
struct StatusError(String);

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StatusError {}

struct Gateway {
    host: String,
    email: String,
    password: Option<String>,
    token: Option<String>,
    /// Set when the gateway says there have been too many login attempts.
    no_login_until: Option<DateTime<Utc>>,
}

impl Gateway {
    fn url(&self, endpoint: &str) -> String {
        format!("https://{}/api/{endpoint}", self.host)
    }

    fn login(&mut self, client: &mut Client<EspHttpConnection>) -> anyhow::Result<()> {
        let Some(password) = &self.password else {
            anyhow::bail!("there is no Tesla password, set it on the config page");
        };
        if let Some(until) = self.no_login_until.filter(|until| *until > Utc::now()) {
            anyhow::bail!("not logging in until {}, after too many attempts", format_time(until));
        }
        info!("Tesla gateway POST login/Basic");
        // force_sm_off is always false, as true stops the system on some versions of the gateway's software.
        let body = serde_json::to_vec(&json!({
            "username": "customer",
            "email": self.email,
            "password": password,
            "force_sm_off": false,
        }))?;
        let length = body.len().to_string();
        let headers = [("Content-Type", "application/json"), ("Content-Length", length.as_str())];
        let url = self.url("login/Basic");
        let mut request = client.post(&url, &headers)?;
        request.write_all(&body)?;
        let mut response = request.submit()?;
        let status = response.status();
        let body = read_body(&mut response)?;
        match status {
            401 | 403 => return Err(PasswordRejected.into()),
            429 => {
                self.no_login_until = Some(Utc::now() + LOCKOUT_WAIT);
                anyhow::bail!("the gateway has locked out the customer login after too many attempts");
            }
            status if !(200..300).contains(&status) => {
                anyhow::bail!("the gateway responded {status} to login: {}", summary(&body))
            }
            _ => {}
        }
        #[derive(Deserialize)]
        struct LoginResponse {
            token: String,
        }
        self.token = Some(serde_json::from_slice::<LoginResponse>(&body)?.token);
        Ok(())
    }

    fn send_get(&self, client: &mut Client<EspHttpConnection>, endpoint: &str) -> anyhow::Result<(u16, Vec<u8>)> {
        log::debug!("Tesla gateway GET {endpoint}");
        let authorization = format!("Bearer {}", self.token.as_deref().unwrap_or_default());
        let headers = [("Authorization", authorization.as_str())];
        let url = self.url(endpoint);
        let mut response = client.request(Method::Get, &url, &headers)?.submit()?;
        let status = response.status();
        Ok((status, read_body(&mut response)?))
    }

    /// GETs endpoint, logging in again if the token is rejected, unless this poll has already logged in, see poll.
    fn get(&mut self, client: &mut Client<EspHttpConnection>, endpoint: &str, logged_in: &mut bool) -> anyhow::Result<Vec<u8>> {
        let (mut status, mut body) = self.send_get(client, endpoint)?;
        if matches!(status, 401 | 403) && !*logged_in {
            info!("The Tesla gateway did not accept the token, logging in again");
            self.token = None;
            *logged_in = true;
            self.login(client)?;
            (status, body) = self.send_get(client, endpoint)?;
        }
        if !(200..300).contains(&status) {
            return Err(StatusError(format!("the gateway responded {status} to {endpoint}: {}", summary(&body))).into());
        }
        Ok(body)
    }
}

fn read_body(response: &mut embedded_svc::http::client::Response<&mut EspHttpConnection>) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut buffer = [0u8; 512];
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            return Ok(body);
        }
        if body.len() + read > MAX_BODY {
            anyhow::bail!("the response is longer than {MAX_BODY} bytes");
        }
        body.extend_from_slice(&buffer[..read]);
    }
}

/// The start of a response, for an error message.
fn summary(body: &[u8]) -> String {
    String::from_utf8_lossy(&body[..body.len().min(100)]).into_owned()
}

struct PollTask {
    gateway: Gateway,
    store: Arc<Mutex<Snapshots>>,
    panel: Arc<Panel>,
    slots: Slots,
    /// The connection to the gateway, kept between polls, so that each is a few requests, not a new HTTPS
    /// connection, which takes a second and most of the RAM which is free. None until the first, after it failed, or
    /// after a poll which needed the RAM closed it, see KeptConnection::release.
    client: KeptConnection,
}

impl ScheduledTask<Esp32Platform> for PollTask {
    fn run(&mut self, sparko: &mut Esp32Platform) -> anyhow::Result<()> {
        let slot = self.slots.current()?;
        self.poll(sparko, slot);
        Ok(())
    }

    fn name(&self) -> &str {
        "Tesla Capture"
    }
}

impl PollTask {
    /// Polls the gateway, saving the record, or the error if the gateway could not be reached, for the screen.
    fn poll(&mut self, sparko: &mut Esp32Platform, slot: Option<DateTime<Utc>>) {
        let requested = Utc::now();
        let result = self.fetch(slot, requested);
        let heap = unsafe { esp_idf_sys::esp_get_free_heap_size() };
        let minimum_heap = unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() };
        let mut store = self.store.lock().unwrap();
        match result {
            Ok(record) => {
                store.add(record);
                info!("Tesla poll saved, free heap {heap}, minimum {minimum_heap}");
            }
            Err(error) => {
                if error.is::<PasswordRejected>() {
                    self.gateway.password = None;
                    if let Err(error) = forget_password(sparko) {
                        log::error!("Unable to remove the rejected Tesla password from the config, {error:#}");
                    }
                }
                warn!("Tesla poll failed, {error:#}");
                store.failed(requested, format!("{error:#}"));
            }
        }
        set_panel(&self.panel, &store);
    }

    /// Requests each of the ENDPOINTS in turn, on one connection, logging in first if there is no token.
    /// On the connection kept from the last poll, if there is one, and if that fails, e.g. as the gateway has closed it,
    /// once more on a new one.
    fn fetch(&mut self, slot: Option<DateTime<Utc>>, requested: DateTime<Utc>) -> anyhow::Result<Record> {
        if let Some(mut client) = self.client.take() {
            match self.fetch_on(&mut client, slot, requested) {
                Ok(record) => {
                    self.client.keep(client);
                    return Ok(record);
                }
                Err(error) if error.is::<PasswordRejected>() => return Err(error),
                Err(error) => info!("The kept connection to the Tesla gateway failed, connecting again, {error:#}"),
            }
        }
        let mut client = Client::wrap(EspHttpConnection::new(&Configuration {
            timeout: Some(Duration::from_secs(10)),
            ..Default::default()
        })?);
        let record = self.fetch_on(&mut client, slot, requested)?;
        self.client.keep(client);
        Ok(record)
    }

    fn fetch_on(&mut self, client: &mut Client<EspHttpConnection>, slot: Option<DateTime<Utc>>, requested: DateTime<Utc>)
        -> anyhow::Result<Record> {
        let mut logged_in = false;
        if self.gateway.token.is_none() {
            logged_in = true;
            self.gateway.login(client)?;
        }
        let mut responses = Vec::with_capacity(ENDPOINTS.len());
        for endpoint in ENDPOINTS {
            let response = self
                .gateway
                .get(client, endpoint, &mut logged_in)
                .and_then(|body| Response::parse(endpoint, &body));
            match response {
                Ok(response) => responses.push(Ok(response)),
                // Recorded as the app does, see bruce-smart-home's snapshot::fetch, but anything else, e.g. the gateway
                // not answering, stops the poll, rather than waiting for it to time out on every endpoint.
                Err(error) if error.is::<StatusError>() || error.is::<serde_json::Error>() => {
                    warn!("Unable to fetch {endpoint} from the Tesla gateway, {error:#}");
                    responses.push(Err(format!("{error:#}").into()));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Record { slot, requested, received: Utc::now(), responses })
    }
}

/// Removes the password from the config, so that it is not tried again after a restart.
fn forget_password(sparko: &Esp32Platform) -> anyhow::Result<()> {
    let holder = sparko
        .config_manager
        .features
        .get(FEATURE_NAME)
        .ok_or_else(|| anyhow::anyhow!("there is no config for {FEATURE_NAME}"))?;
    let mut config = holder.inner.lock().unwrap();
    if let Some(value) = config.spec.map.get_mut(PASSWORD) {
        holder.config_store.remove(PASSWORD, value)?;
    }
    Ok(())
}
