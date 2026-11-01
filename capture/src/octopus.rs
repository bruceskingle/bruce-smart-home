//! Polls what Octopus's smart charging (Intelligent Octopus) does, which Octopus only keeps for a few days, as
//! marco-sparko's src/octopus/flex_poll.rs does: the devices' preferences, including whether charging is capped to the
//! hours Octopus guarantees are cheap, the charging it has planned, and the charging it has carried out.
//!
//! As each half hour is either cheap or not, it polls just before and just after each one ends, see DEFAULT_SCHEDULE,
//! and as a connection to Octopus is the costliest thing the device does, each poll is one request, flexPoll, on one
//! connection, once it knows the charging device, see Octopus::poll.
//!
//! Each record is what the app saves, {"slot", "requested", "received", "responses"}, on the SD card, see DayFiles, and
//! the latest in RAM, for the screen, and for the app if there is no card.
//!
//! It only needs the API key: the account is the key's first, unless the config says which.

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
use serde_json::{Value, json};
use sparko_embedded_std::config::ConfigSpec;
use sparko_embedded_std::config::ConfigSpecValue;
use sparko_embedded_std::config::FeatureConfig;
use sparko_embedded_std::config::TypedValue;
use sparko_embedded_std::feature::FeatureDescriptor;
use sparko_embedded_std::platform::PlatformInitializer;
use sparko_embedded_std::task::scheduler::ScheduledTask;
use sparko_esp_idf::{Esp32Platform, Esp32PlatformInitializer, Feature};

use crate::connections::KeptConnection;
use crate::day_files::{self, DayFiles};
use crate::device::{Device, FIRMWARE, captured_by};
use crate::records::format_time;
use crate::screen::{Line, Panel};
use crate::slots::Slots;

/// The feature's name, which is also its NVS namespace, so at most 15 characters.
const FEATURE_NAME: &str = "OctopusCapture";

/// 30 seconds before and after each half hour ends, 30 seconds clear of the other features' polls on the minute.
const DEFAULT_SCHEDULE: &str = "30 0,29,30,59 * * * *";

/// 2 hours of records, about 1.2 KB each, for when there is no SD card.
const RAM_CAPACITY: usize = 8;

const URL: &str = "https://api.octopus.energy/v1/graphql/";

/// How long a token is used for, a little less than the hour Octopus gives it.
const TOKEN_LIFE: Duration = Duration::from_secs(55 * 60);

/// The most requests it makes in an hour, as marco-sparko's flex_poll.rs, so that the Octopus app and others using the
/// account are not affected.
const CALLS_PER_HOUR: usize = 20;

/// The largest response read, to protect the RAM.
const MAX_BODY: usize = 32 * 1024;

/// Where its line goes on the screen, after the zappi's.
const PANEL_ORDER: u8 = 3;

/// The accounts the API key may use, the first of which is polled if the config does not say which.
const ACCOUNTS: &str = "query accounts { viewer { accounts { number } } }";

const LOGIN: &str = "mutation obtainKrakenToken($input: ObtainJSONWebTokenInput!) { obtainKrakenToken(input: $input) { token } }";

/// The devices, to find the charging device the first time, as in marco-sparko's graphql/octopus/flex.graphql.
const DEVICES: &str = "query devicePreferences($accountNumber: String!) { devices(accountNumber: $accountNumber) { id __typename \
    preferences { __typename mode targetType isChargingDurationCapped } } }";

/// Everything polled, in one request, the same as flexPoll in marco-sparko's graphql/octopus/flex.graphql.
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
    /// Closed before each poll, for the RAM, see KeptConnection::release.
    kept: KeptConnection,
}

impl OctopusCapture {
    pub fn new(panel: Arc<Panel>, kept: KeptConnection) -> Self {
        Self { panel, kept }
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
        let dir = sparko.sd_card().map(|mount_point| std::path::Path::new(mount_point).join("capture").join("octopus"));
        let store = Arc::new(Mutex::new(Store {
            files: DayFiles::new(dir),
            ram: VecDeque::with_capacity(RAM_CAPACITY),
            line: ("Octopus: not polled yet".to_string(), false),
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
            panel: self.panel.clone(),
            kept: self.kept.clone(),
            slots: Slots::new(&schedule_spec)?,
            captured_by: captured_by(sparko),
        };
        // A first poll now, as the app does, so there is something to show without waiting for the first slot.
        poll.poll(None);
        initializer.add_task(Box::new(poll), &schedule_spec)?;

        sparko.api_get("/api/capture/octopus/records", move |request| {
            let (files, device_json) = {
                let store = store.lock().unwrap();
                (store.files.usable(), device.to_json(&store.files, store.ram.len(), RAM_CAPACITY))
            };
            day_files::respond(request, &device_json, files, |since, write| {
                let store = store.lock().unwrap();
                store
                    .ram
                    .iter()
                    .filter(|(time, _)| since.is_none_or(|since| *time > since))
                    .try_for_each(|(_, line)| write(line.as_bytes()))
            })
        })?;
        Ok(())
    }
}

struct Store {
    files: DayFiles,
    /// The latest records, oldest first, by when each is of, as their JSON.
    ram: VecDeque<(DateTime<Utc>, Box<str>)>,
    /// What the screen shows, see panel_line.
    line: Line,
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

    /// POSTs a GraphQL operation, returning its data, or its errors.
    fn graphql(
        &mut self,
        client: &mut Client<EspHttpConnection>,
        query: &str,
        variables: Value,
        token: Option<&str>,
    ) -> anyhow::Result<Value> {
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
        let mut json: Value = serde_json::from_slice(&body)?;
        if let Some(errors) = json.get("errors").and_then(|errors| errors.as_array()).filter(|errors| !errors.is_empty()) {
            let messages: Vec<String> = errors
                .iter()
                .map(|error| {
                    let code = error["extensions"]["errorCode"].as_str().unwrap_or_default();
                    format!("{} {code}", error["message"].as_str().unwrap_or("error"))
                })
                .collect();
            anyhow::bail!("{}", messages.join("; "));
        }
        Ok(json["data"].take())
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
        let data = self.graphql(client, LOGIN, variables, None)?;
        let token = data["obtainKrakenToken"]["token"]
            .as_str()
            .ok_or_else(|| anyhow!("Octopus gave no token"))?
            .to_string();
        self.token = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    /// The responses, as marco-sparko's flex_poll::poll saves them: one flexPoll request once the charging device is
    /// known, otherwise the devices first, to find it.
    fn poll(&mut self) -> anyhow::Result<Value> {
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
                let data = self.graphql(&mut client, ACCOUNTS, json!({}), Some(&token))?;
                let account = data["viewer"]["accounts"][0]["number"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Octopus has no account for the API key"))?
                    .to_string();
                self.account = Some(account.clone());
                account
            }
        };
        if self.charging_device.is_none() {
            info!("Octopus finding the charging device");
            let data = self.graphql(&mut client, DEVICES, json!({ "accountNumber": account }), Some(&token))?;
            self.charging_device = charging_device(&data["devices"]);
        }
        let Some(device_id) = self.charging_device.clone() else {
            anyhow::bail!("Octopus has no charging device on the account");
        };
        info!("Octopus flexPoll");
        let variables = json!({ "accountNumber": account, "deviceId": device_id });
        let mut data = self
            .graphql(&mut client, FLEX_POLL, variables, Some(&token))
            // e.g. the device has gone, so that the next poll finds the devices again.
            .inspect_err(|_| self.charging_device = None)?;
        let devices = data["devices"].take();
        // The device may have changed, so that the next poll asks for the new one.
        self.charging_device = charging_device(&devices);
        Ok(json!({
            "devicePreferences": { "devices": devices },
            "flexPlannedDispatches": { device_id: { "flexPlannedDispatches": data["flexPlannedDispatches"].take() } },
            "completedDispatches": { "completedDispatches": data["completedDispatches"].take() },
        }))
    }
}

/// The first device which charges, a charge point or vehicle, the only ones with planned dispatches.
fn charging_device(devices: &Value) -> Option<String> {
    devices
        .as_array()?
        .iter()
        .find(|device| matches!(device["__typename"].as_str(), Some("SmartFlexChargePoint" | "SmartFlexVehicle")))
        .and_then(|device| device["id"].as_str().map(String::from))
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
fn panel_line(responses: &Value) -> Line {
    let now = Utc::now();
    let next = responses["flexPlannedDispatches"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, device)| device["flexPlannedDispatches"].as_array())
        .flatten()
        .filter_map(|dispatch| {
            let start = DateTime::parse_from_rfc3339(dispatch["start"].as_str()?).ok()?.with_timezone(&Utc);
            let end = DateTime::parse_from_rfc3339(dispatch["end"].as_str()?).ok()?.with_timezone(&Utc);
            (end > now).then_some(start)
        })
        .min();
    let capped = responses["devicePreferences"]["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|device| device["preferences"]["isChargingDurationCapped"].as_str())
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
            Ok(responses) => {
                store.line = panel_line(&responses);
                if store.files.problem().is_some() {
                    store.line.0.push_str(", no SD");
                    store.line.1 = true;
                }
                let record = json!({
                    "slot": slot.map(format_time),
                    "requested": format_time(requested),
                    "received": format_time(received),
                    "capturedBy": self.captured_by,
                    "firmware": FIRMWARE,
                    "responses": responses,
                });
                let time = slot.unwrap_or(requested);
                store.files.append(time, &record);
                let Store { files, ram, .. } = &mut *store;
                files.keep(ram, RAM_CAPACITY, time, &record);
                info!("Octopus poll saved, free heap {heap}, minimum {minimum_heap}");
            }
            Err(error) => {
                warn!("Octopus poll failed, {error:#}");
                store.line = (format!("Octopus: {error:#}"), true);
            }
        }
        self.panel.set(PANEL_ORDER, vec![store.line.clone()]);
    }
}
