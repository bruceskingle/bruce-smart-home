//! Polls what Octopus's smart charging (Intelligent Octopus) does, which Octopus only keeps for a few days, as
//! bruce-smart-home's src/octopus/flex_poll.rs does: the devices' preferences, including whether charging is capped to the
//! hours Octopus guarantees are cheap, the charging it has planned, and the charging it has carried out.
//!
//! As each half hour is either cheap or not, it polls just before and just after each one ends, see DEFAULT_SCHEDULE,
//! and as a connection to Octopus is the costliest thing the device does, each poll is one request, flexPoll, on one
//! connection, once it knows the charging device, see Octopus::poll.
//!
//! Each record is what the app saves, {"slot", "requested", "received", "responses"}, put in the store as the app would
//! put it, see db.rs, by the account, which is why the records kept before the store are converted after the first
//! poll, once it is known, see PollTask::poll.
//!
//! It only needs the API key: the account is the key's first, unless the config says which.
//!
//! The responses are kept as the JSON Octopus sends, see RawValue, with only what the device needs read from them, as
//! a serde_json::Value of them takes several times the RAM of their JSON: each object a BTreeMap of at least 400 bytes,
//! and a night's charging is a completed dispatch, two objects, for each half hour, which ran the CYD out of memory.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use chrono::{DateTime, Local, Utc};
use croner::Cron;
use embedded_svc::http::client::Client;
use embedded_svc::io::Write;
use esp_idf_svc::http::Method;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use log::{info, warn};
use serde::Deserialize;
use serde_json::json;
use serde_json::value::RawValue;
use bruce_embedded_std::config::ConfigSpec;
use bruce_embedded_std::config::ConfigSpecValue;
use bruce_embedded_std::config::FeatureConfig;
use bruce_embedded_std::config::TypedValue;
use bruce_embedded_std::feature::FeatureDescriptor;
use bruce_embedded_std::platform::PlatformInitializer;
use bruce_embedded_std::task::scheduler::ScheduledTask;
use bruce_esp_idf::{Esp32Platform, Esp32PlatformInitializer, Feature};

use crate::connections::KeptConnection;
use crate::db::Db;
use crate::device::{Device, FIRMWARE, captured_by};
use crate::records::{LastPoll, format_time};
use crate::screen::{Line, Panel};
use crate::slots::Slots;

/// The feature's name, which is also its NVS namespace, so at most 15 characters.
const FEATURE_NAME: &str = "OctopusCapture";

/// 30 seconds before and after each half hour ends, 30 seconds clear of the other features' polls on the minute.
const DEFAULT_SCHEDULE: &str = "30 0,29,30,59 * * * *";

const URL: &str = "https://api.octopus.energy/v1/graphql/";

/// How long a token is used for, a little less than the hour Octopus gives it.
const TOKEN_LIFE: Duration = Duration::from_secs(55 * 60);

/// The most requests it makes in an hour, as bruce-smart-home's flex_poll.rs, so that the Octopus app and others using the
/// account are not affected.
const CALLS_PER_HOUR: usize = 20;

/// The largest response read, to protect the RAM.
const MAX_BODY: usize = 32 * 1024;

/// Where its line goes on the screen, after the zappi's.
const PANEL_ORDER: u8 = 3;

/// The accounts the API key may use, the first of which is polled if the config does not say which.
const ACCOUNTS: &str = "query accounts { viewer { accounts { number } } }";

const LOGIN: &str = "mutation obtainKrakenToken($input: ObtainJSONWebTokenInput!) { obtainKrakenToken(input: $input) { token } }";

/// The devices, to find the charging device the first time, as in bruce-smart-home's graphql/octopus/flex.graphql.
const DEVICES: &str = "query devicePreferences($accountNumber: String!) { devices(accountNumber: $accountNumber) { id __typename \
    preferences { __typename mode targetType isChargingDurationCapped } } }";

/// Everything polled, in one request, the same as flexPoll in bruce-smart-home's graphql/octopus/flex.graphql.
const FLEX_POLL: &str = "query flexPoll($accountNumber: String!, $deviceId: String!) { \
    devices(accountNumber: $accountNumber) { id __typename preferences { __typename mode targetType isChargingDurationCapped } } \
    flexPlannedDispatches(deviceId: $deviceId) { start end type energyAddedKwh } \
    completedDispatches(accountNumber: $accountNumber) { start end delta meta { source location } } }";

/// The config's names must differ from every other feature's, and be at most 15 characters, see tesla.rs.
#[derive(FeatureConfig)]
#[config(label = "Octopus Smart Charging")]
pub struct OctopusConfig {
    /// e.g. A-1234ABCD, which is only needed if the API key's user has more than one account.
    #[config(len = 16, label = "Octopus Account Number (the first account if empty)")]
    octopus_account: Option<String>,
    #[config(len = 64, secret, label = "Octopus API Key")]
    octopus_key: String,
    #[config(label = "Octopus Poll Schedule (cron, seconds first; before and after each half hour if empty)")]
    octopus_sched: Option<Cron>,
}

pub struct OctopusCapture {
    panel: Arc<Panel>,
    db: Arc<Db>,
    /// Closed before each poll, for the RAM, see KeptConnection::release.
    kept: KeptConnection,
}

impl OctopusCapture {
    pub fn new(panel: Arc<Panel>, db: Arc<Db>, kept: KeptConnection) -> Self {
        Self { panel, db, kept }
    }
}

impl Feature for OctopusCapture {
    type Config = OctopusConfig;

    fn init(&self, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<FeatureDescriptor> {
        self.panel.start(initializer)?;
        Ok(FeatureDescriptor { name: FEATURE_NAME.to_string(), config: OctopusConfig::to_config_spec()? })
    }

    fn start(
        &mut self,
        sparko: &mut Esp32Platform,
        initializer: &mut Esp32PlatformInitializer,
        config: OctopusConfig,
    ) -> anyhow::Result<()> {
        let schedule_spec = config
            .octopus_sched
            .map(|schedule| schedule.pattern.to_string())
            .unwrap_or_else(|| DEFAULT_SCHEDULE.to_string());
        let store = Arc::new(Mutex::new(Store {
            line: ("Octopus: not polled yet".to_string(), false),
            last_poll: None,
        }));
        let device = Arc::new(Device::new(&schedule_spec));

        let mut poll = PollTask {
            octopus: Octopus {
                account: config.octopus_account,
                key: config.octopus_key,
                token: None,
                charging_device: None,
                calls: VecDeque::new(),
            },
            store: store.clone(),
            db: self.db.clone(),
            converted: false,
            panel: self.panel.clone(),
            kept: self.kept.clone(),
            slots: Slots::new(&schedule_spec)?,
            captured_by: captured_by(sparko),
        };
        // A first poll now, as the app does, so there is something to show without waiting for the first slot.
        poll.poll(None);
        initializer.add_task(Box::new(poll), &schedule_spec)?;

        // For the app, at /api/store/info, see db.rs.
        self.db.add_status("octopus", move |db| device.to_json(db, store.lock().unwrap().last_poll.as_ref()));
        Ok(())
    }
}

struct Store {
    /// What the screen shows, see panel_line.
    line: Line,
    /// How the latest poll went, for the app, see Device::to_json.
    last_poll: Option<LastPoll>,
}

/// The Octopus API, see poll.
struct Octopus {
    /// From the config, or the API key's first account, see account.
    account: Option<String>,
    key: String,
    /// The token from logging in with the API key, and when it was given.
    token: Option<(String, Instant)>,
    /// The device whose planned dispatches flexPoll asks for, from an earlier poll's devices.
    charging_device: Option<String>,
    /// When the requests of the last hour were made, see spend.
    calls: VecDeque<Instant>,
}

impl Octopus {
    /// Counts a request, or fails if CALLS_PER_HOUR have been made in the last hour.
    fn spend(&mut self) -> anyhow::Result<()> {
        while self.calls.front().is_some_and(|call| call.elapsed() >= Duration::from_secs(60 * 60)) {
            self.calls.pop_front();
        }
        if self.calls.len() >= CALLS_PER_HOUR {
            return Err(anyhow!("skipped, {CALLS_PER_HOUR} requests to Octopus in the last hour"));
        }
        self.calls.push_back(Instant::now());
        Ok(())
    }

    /// POSTs a GraphQL operation, returning the response, whose data is read with data, or its errors.
    fn graphql(
        &mut self,
        client: &mut Client<EspHttpConnection>,
        query: &str,
        variables: serde_json::Value,
        token: Option<&str>,
    ) -> anyhow::Result<Vec<u8>> {
        self.spend()?;
        let body = serde_json::to_vec(&json!({ "query": query, "variables": variables }))?;
        let length = body.len().to_string();
        let mut headers = vec![("Content-Type", "application/json"), ("Content-Length", length.as_str())];
        if let Some(token) = token {
            headers.push(("Authorization", token));
        }
        let mut request = client.request(Method::Post, URL, &headers)?;
        request.write_all(&body)?;
        let mut response = request.submit()?;
        let status = response.status();
        let body = read_body(&mut response)?;
        if !(200..300).contains(&status) {
            anyhow::bail!("Octopus responded {status}: {}", String::from_utf8_lossy(&body[..body.len().min(100)]));
        }
        let errors: Errors = serde_json::from_slice(&body)?;
        if !errors.errors.is_empty() {
            let messages: Vec<String> = errors
                .errors
                .iter()
                .map(|error| {
                    let code = error.extensions.as_ref().and_then(|extensions| extensions.error_code.as_deref());
                    format!("{} {}", error.message.as_deref().unwrap_or("error"), code.unwrap_or_default())
                })
                .collect();
            anyhow::bail!("{}", messages.join("; "));
        }
        Ok(body)
    }

    /// The token, logging in with the API key first if there is none, or it is old.
    fn token(&mut self, client: &mut Client<EspHttpConnection>) -> anyhow::Result<String> {
        if let Some((token, given)) = &self.token {
            if given.elapsed() < TOKEN_LIFE {
                return Ok(token.clone());
            }
        }
        info!("Octopus logging in");
        let variables = json!({ "input": { "APIKey": self.key } });
        let body = self.graphql(client, LOGIN, variables, None)?;
        let token = data::<Login>(&body)?
            .obtain_kraken_token
            .and_then(|token| token.token)
            .ok_or_else(|| anyhow!("Octopus gave no token"))?;
        self.token = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    /// The responses, as bruce-smart-home's flex_poll::poll saves them, as their JSON, and what the screen shows of them:
    /// one flexPoll request once the charging device is known, otherwise the devices first, to find it.
    fn poll(&mut self) -> anyhow::Result<(String, Line)> {
        crate::device::log_memory("Before Octopus");
        let mut client = Client::wrap(EspHttpConnection::new(&Configuration {
            timeout: Some(Duration::from_secs(20)),
            // Room for the Authorization header, the token, a JWT of about 1 KB, as the default is 512 bytes.
            buffer_size_tx: Some(2048),
            crt_bundle_attach: Some(esp_idf_sys::esp_crt_bundle_attach),
            ..Default::default()
        })?);
        let token = self.token(&mut client)?;
        let account = match &self.account {
            Some(account) => account.clone(),
            None => {
                info!("Octopus finding the account");
                let body = self.graphql(&mut client, ACCOUNTS, json!({}), Some(&token))?;
                let account = data::<Accounts>(&body)?
                    .viewer
                    .and_then(|viewer| viewer.accounts?.into_iter().next()?.number)
                    .ok_or_else(|| anyhow!("Octopus has no account for the API key"))?;
                self.account = Some(account.clone());
                account
            }
        };
        if self.charging_device.is_none() {
            info!("Octopus finding the charging device");
            let body = self.graphql(&mut client, DEVICES, json!({ "accountNumber": account }), Some(&token))?;
            self.charging_device = charging_device(&devices(data::<DevicesData>(&body)?.devices)?);
        }
        let Some(device_id) = self.charging_device.clone() else {
            anyhow::bail!("Octopus has no charging device on the account");
        };
        info!("Octopus flexPoll");
        let variables = json!({ "accountNumber": account, "deviceId": device_id });
        let body = self
            .graphql(&mut client, FLEX_POLL, variables, Some(&token))
            // e.g. the device has gone, so that the next poll finds the devices again.
            .inspect_err(|_| self.charging_device = None)?;
        drop(client);
        let data = data::<FlexPoll>(&body)?;
        let devices_read = devices(data.devices)?;
        // The device may have changed, so that the next poll asks for the new one.
        self.charging_device = charging_device(&devices_read);
        let planned: Option<Vec<Dispatch>> = serde_json::from_str(data.flex_planned_dispatches.get())?;
        let line = panel_line(&devices_read, planned.as_deref().unwrap_or_default());
        let responses = format!(
            r#"{{"devicePreferences":{{"devices":{}}},"flexPlannedDispatches":{{{}:{{"flexPlannedDispatches":{}}}}},"completedDispatches":{{"completedDispatches":{}}}}}"#,
            data.devices.get(),
            serde_json::to_string(&device_id)?,
            data.flex_planned_dispatches.get(),
            data.completed_dispatches.get()
        );
        Ok((responses, line))
    }
}

/// A response's errors, see Octopus::graphql.
#[derive(Deserialize)]
struct Errors {
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: Option<String>,
    extensions: Option<Extensions>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Extensions {
    error_code: Option<String>,
}

/// A response's data, as T, which the operation's types below read only what the device needs of.
fn data<'a, T: Deserialize<'a>>(body: &'a [u8]) -> anyhow::Result<T> {
    #[derive(Deserialize)]
    struct Data<T> {
        data: Option<T>,
    }
    serde_json::from_slice::<Data<T>>(body)?.data.ok_or_else(|| anyhow!("Octopus gave no data"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Login {
    obtain_kraken_token: Option<Token>,
}

#[derive(Deserialize)]
struct Token {
    token: Option<String>,
}

#[derive(Deserialize)]
struct Accounts {
    viewer: Option<Viewer>,
}

#[derive(Deserialize)]
struct Viewer {
    accounts: Option<Vec<Account>>,
}

#[derive(Deserialize)]
struct Account {
    number: Option<String>,
}

#[derive(Deserialize)]
struct DevicesData<'a> {
    #[serde(borrow)]
    devices: &'a RawValue,
}

/// flexPoll's data, as the JSON Octopus sent, see the module comment.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlexPoll<'a> {
    #[serde(borrow)]
    devices: &'a RawValue,
    #[serde(borrow)]
    flex_planned_dispatches: &'a RawValue,
    #[serde(borrow)]
    completed_dispatches: &'a RawValue,
}

/// What the device reads of each of the devices.
#[derive(Deserialize)]
struct DeviceRead {
    id: Option<String>,
    #[serde(rename = "__typename")]
    typename: Option<String>,
    preferences: Option<Preferences>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    is_charging_duration_capped: Option<String>,
}

/// What the device reads of each planned dispatch.
#[derive(Deserialize)]
struct Dispatch {
    start: Option<String>,
    end: Option<String>,
}

/// The devices, of which there are none if Octopus gives null.
fn devices(devices: &RawValue) -> anyhow::Result<Vec<DeviceRead>> {
    let devices: Option<Vec<DeviceRead>> = serde_json::from_str(devices.get())?;
    Ok(devices.unwrap_or_default())
}

/// The first device which charges, a charge point or vehicle, the only ones with planned dispatches.
fn charging_device(devices: &[DeviceRead]) -> Option<String> {
    devices
        .iter()
        .find(|device| matches!(device.typename.as_deref(), Some("SmartFlexChargePoint" | "SmartFlexVehicle")))
        .and_then(|device| device.id.clone())
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

/// What the responses say for the screen: when the next planned charging starts, or none, and whether charging is
/// capped to the cheap hours, e.g. "Octopus 23:30 capped".
fn panel_line(devices: &[DeviceRead], planned: &[Dispatch]) -> Line {
    let now = Utc::now();
    let next = planned
        .iter()
        .filter_map(|dispatch| {
            let start = DateTime::parse_from_rfc3339(dispatch.start.as_deref()?).ok()?.with_timezone(&Utc);
            let end = DateTime::parse_from_rfc3339(dispatch.end.as_deref()?).ok()?.with_timezone(&Utc);
            (end > now).then_some(start)
        })
        .min();
    let capped = devices
        .iter()
        .filter_map(|device| device.preferences.as_ref()?.is_charging_duration_capped.as_deref())
        .find(|cap| *cap != "NOT_APPLICABLE");
    let cap = match capped {
        Some("ENABLED") => " capped",
        Some("DISABLED") => " NOT capped",
        _ => "",
    };
    let next = match next {
        Some(start) if start <= now => "now".to_string(),
        Some(start) => start.with_timezone(&Local).format("%H:%M").to_string(),
        None => "none".to_string(),
    };
    // Not capped means charging outside the cheap hours is at the peak rate, so worth noticing.
    (format!("Octopus {next}{cap}"), capped == Some("DISABLED"))
}

struct PollTask {
    octopus: Octopus,
    store: Arc<Mutex<Store>>,
    db: Arc<Db>,
    /// Whether the records kept before the store have been converted, see poll.
    converted: bool,
    panel: Arc<Panel>,
    /// Closed before each poll, for the RAM, see KeptConnection::release.
    kept: KeptConnection,
    slots: Slots,
    /// In each record's capturedBy, see device::captured_by.
    captured_by: String,
}

impl ScheduledTask<Esp32Platform> for PollTask {
    fn run(&mut self, _sparko: &mut Esp32Platform) -> anyhow::Result<()> {
        let slot = self.slots.current()?;
        self.poll(slot);
        Ok(())
    }

    fn name(&self) -> &str {
        "Octopus Capture"
    }
}

impl PollTask {
    /// Polls, saving the record, or showing the error.
    fn poll(&mut self, slot: Option<DateTime<Utc>>) {
        self.kept.release();
        let requested = Utc::now();
        let result = self.octopus.poll();
        let received = Utc::now();
        let heap = unsafe { esp_idf_sys::esp_get_free_heap_size() };
        let minimum_heap = unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() };
        let mut store = self.store.lock().unwrap();
        match result {
            Ok((responses, line)) => {
                store.line = line;
                let mut record = json!({
                    "slot": slot.map(format_time),
                    "requested": format_time(requested),
                    "received": format_time(received),
                    "capturedBy": self.captured_by,
                    "firmware": FIRMWARE,
                })
                .to_string();
                // The responses as their JSON, last, in place of the closing brace, see the module comment.
                record.pop();
                record.push_str(r#","responses":"#);
                record.push_str(&responses);
                record.push('}');
                drop(responses);
                // The account, which the poll found if the config did not say.
                let account = self.octopus.account.clone().unwrap_or_default();
                let put = serde_json::from_str::<serde_json::Value>(&record).map_err(anyhow::Error::from).map(|record| {
                    self.db.put(|store, writer| Ok(Some(crate::device_kinds::octopus_flex(store, &account, &record, writer)?)))
                });
                if let Err(error) = &put {
                    warn!("Unable to read the Octopus record, {error:#}");
                }
                if !put.unwrap_or(false) && self.db.problem().is_some() {
                    store.line.0.push_str(", not stored");
                    store.line.1 = true;
                }
                // In a thread of its own, as the other features' are, see db.rs, as it needs more stack than the
                // scheduler's task has, and takes a while.
                if !self.converted {
                    self.converted = true;
                    let (db, panel) = (self.db.clone(), self.panel.clone());
                    let converting = std::thread::Builder::new().stack_size(16 * 1024).spawn(move || {
                        db.convert(&panel, "octopus", &|store, writer, record| {
                            Ok(Some(crate::device_kinds::octopus_flex(store, &account, record, writer)?))
                        });
                    });
                    if let Err(error) = converting {
                        warn!("Unable to start converting the old Octopus records, {error:#}");
                        self.converted = false;
                    }
                }
                store.last_poll = Some(LastPoll::succeeded(received));
                info!("Octopus poll saved, free heap {heap}, minimum {minimum_heap}");
            }
            Err(error) => {
                warn!("Octopus poll failed, {error:#}");
                store.last_poll = Some(LastPoll::failed(store.last_poll.as_ref(), received, format!("{error:#}")));
                store.line = (format!("Octopus: {error:#}"), true);
            }
        }
        self.panel.set(PANEL_ORDER, vec![store.line.clone()]);
    }
}
