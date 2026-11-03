//! Polls myenergi's status of the hub's devices, cgi-jstatus-*, as marco-sparko's src/myenergi does, for what myenergi
//! does not keep: what each zappi is doing now, e.g. the charge added since the car was plugged in, which is only kept
//! until it is next plugged in, so polling often enough catches every charging session's total. The app fetches each
//! device's minute records itself, as myenergi keeps those.
//!
//! Each record is what the app saves, {"slot", "requested", "received", "response"}, with only what the app uses of the
//! response, see reduce, on the SD card, see DayFiles, and the latest in RAM, for the screen, and for the app if there
//! is no card.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use croner::Cron;
use embedded_svc::http::client::Client;
use esp_idf_svc::http::Method;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use log::{info, warn};
use md5::{Digest, Md5};
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
const FEATURE_NAME: &str = "MyenergiCapture";

/// Every 5 minutes, on the minute, as the app polls, see marco-sparko's src/polling.rs.
const DEFAULT_SCHEDULE: &str = "0 */5 * * * *";

/// 4 hours of 5 minute records, a few hundred bytes each, for when there is no SD card.
const RAM_CAPACITY: usize = 48;

/// The server which says which server has the hub's data, see Myenergi::host.
const DIRECTOR_HOST: &str = "director.myenergi.net";
const ASN_HEADER: &str = "x_myenergi-asn";

/// The largest response read, to protect the RAM.
const MAX_BODY: usize = 32 * 1024;

/// Where its line goes on the screen, after the Powerwall's.
const PANEL_ORDER: u8 = 2;

/// The fields of each zappi kept, those marco-sparko's StatusSample and ct_names use. Of the other devices only the
/// serial number is kept, sno, so that the app can fetch their minute records.
const ZAPPI_FIELDS: &[&str] = &[
    "sno", "dat", "tim", "pst", "sta", "zmo", "che", "div", "grd", "gen", "ectp1", "ectp2", "ectp3", "ectp4", "ectt1",
    "ectt2", "ectt3", "ectt4", "vol", "frq",
];
const DEVICE_KINDS: &[&str] = &["zappi", "eddi", "harvi", "libbi"];

/// The config's names must differ from every other feature's, and be at most 15 characters, see tesla.rs.
#[derive(FeatureConfig)]
#[config(label = "Myenergi zappi")]
pub struct MyenergiConfig {
    /// The serial number of the myenergi hub, as in the myenergi app.
    #[config(len = 16, label = "Myenergi Hub Serial Number")]
    myenergi_hub: String,
    /// The hub's API key, made in the myenergi app.
    #[config(len = 64, secret, label = "Myenergi API Key")]
    myenergi_key: String,
    #[config(label = "Myenergi Poll Schedule (cron, seconds first; every 5 minutes if empty)")]
    myenergi_sched: Option<Cron>,
}

pub struct MyenergiCapture {
    panel: Arc<Panel>,
    /// Closed before each poll, for the RAM, see KeptConnection::release.
    kept: KeptConnection,
}

impl MyenergiCapture {
    pub fn new(panel: Arc<Panel>, kept: KeptConnection) -> Self {
        Self { panel, kept }
    }
}

impl Feature for MyenergiCapture {
    type Config = MyenergiConfig;

    fn init(&self, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<FeatureDescriptor> {
        self.panel.start(initializer)?;
        Ok(FeatureDescriptor { name: FEATURE_NAME.to_string(), config: MyenergiConfig::to_config_spec()? })
    }

    fn start(
        &mut self,
        sparko: &mut Esp32Platform,
        initializer: &mut Esp32PlatformInitializer,
        config: MyenergiConfig,
    ) -> anyhow::Result<()> {
        let schedule_spec = config
            .myenergi_sched
            .map(|schedule| schedule.pattern.to_string())
            .unwrap_or_else(|| DEFAULT_SCHEDULE.to_string());
        let dir = sparko.sd_card().map(|mount_point| std::path::Path::new(mount_point).join("capture").join("myenergi"));
        let store = Arc::new(Mutex::new(Store {
            files: DayFiles::new(dir),
            ram: VecDeque::with_capacity(RAM_CAPACITY),
            line: ("myenergi: not polled yet".to_string(), false),
        }));
        let device = Arc::new(Device::new(&schedule_spec));

        let mut poll = PollTask {
            myenergi: Myenergi {
                hub: config.myenergi_hub,
                key: config.myenergi_key,
                host: DIRECTOR_HOST.to_string(),
                count: 0,
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

        sparko.api_get("/api/capture/myenergi/records", move |request| {
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

/// The myenergi API, which needs digest authentication, with the hub's serial number and API key.
struct Myenergi {
    hub: String,
    key: String,
    /// The server with the hub's data, the director until it has said which, in the ASN_HEADER of a response.
    host: String,
    /// The digest requests made so far, to number them, see digest_authorization.
    count: u32,
}

impl Myenergi {
    /// GETs path, answering the server's digest challenge, on one connection. A request without authentication gets
    /// the challenge, as the nonce may only be good for one request, as marco-sparko's MyenergiClient::get does.
    fn get(&mut self, path: &str) -> anyhow::Result<Value> {
        crate::device::log_memory("Before myenergi");
        let mut client = Client::wrap(EspHttpConnection::new(&Configuration {
            timeout: Some(Duration::from_secs(15)),
            crt_bundle_attach: Some(esp_idf_sys::esp_crt_bundle_attach),
            ..Default::default()
        })?);
        let url = format!("https://{}{path}", self.host);
        info!("myenergi GET {path}");
        let mut exchange = send(&mut client, &url, &[])?;
        if exchange.status == 401 {
            let challenge = exchange
                .challenge
                .as_deref()
                .and_then(parse_challenge)
                .ok_or_else(|| anyhow!("myenergi asked for authentication, but not digest authentication"))?;
            self.count += 1;
            let mut cnonce = [0u8; 8];
            unsafe { esp_idf_sys::esp_fill_random(cnonce.as_mut_ptr() as *mut _, cnonce.len()) };
            let cnonce: String = cnonce.iter().map(|byte| format!("{byte:02x}")).collect();
            let authorization = digest_authorization(&challenge, &self.hub, &self.key, "GET", path, self.count, &cnonce);
            exchange = send(&mut client, &url, &[("Authorization", &authorization)])?;
        }
        if let Some(asn) = exchange.asn.filter(|asn| !asn.is_empty() && *asn != self.host) {
            info!("myenergi data is on {asn}");
            self.host = asn;
        }
        match exchange.status {
            401 => Err(anyhow!("myenergi did not accept the hub serial number and API key")),
            status if !(200..300).contains(&status) => Err(anyhow!(
                "myenergi responded {status}: {}",
                String::from_utf8_lossy(&exchange.body[..exchange.body.len().min(100)])
            )),
            _ => Ok(serde_json::from_slice(&exchange.body)?),
        }
    }
}

/// What a request got back.
struct Exchange {
    status: u16,
    /// The WWW-Authenticate header, if any.
    challenge: Option<String>,
    /// The ASN_HEADER, if any.
    asn: Option<String>,
    body: Vec<u8>,
}

/// GETs url, reading the whole response, so that the connection can be used again.
fn send(client: &mut Client<EspHttpConnection>, url: &str, headers: &[(&str, &str)]) -> anyhow::Result<Exchange> {
    let mut response = client.request(Method::Get, url, headers)?.submit()?;
    let status = response.status();
    let challenge = response.header("WWW-Authenticate").map(String::from);
    let asn = response.header(ASN_HEADER).map(|asn| asn.trim().to_string());
    let body = read_body(&mut response)?;
    Ok(Exchange { status, challenge, asn, body })
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

/// A digest challenge, from a WWW-Authenticate header, as marco-sparko's src/myenergi/digest.rs.
struct Challenge {
    realm: String,
    nonce: String,
    qop: Option<String>,
    opaque: Option<String>,
}

fn parse_challenge(header: &str) -> Option<Challenge> {
    let params = header.trim().strip_prefix("Digest")?;
    let mut realm = None;
    let mut nonce = None;
    let mut qop = None;
    let mut opaque = None;
    // name="value" or name=value, separated by commas, which quoted values may contain.
    let mut rest = params.trim();
    while !rest.is_empty() {
        let (name, after) = rest.split_once('=')?;
        let after = after.trim_start();
        let (value, after) = match after.strip_prefix('"') {
            Some(quoted) => {
                let end = quoted.find('"')?;
                (quoted[..end].to_string(), &quoted[end + 1..])
            }
            None => {
                let end = after.find(',').unwrap_or(after.len());
                (after[..end].trim().to_string(), &after[end..])
            }
        };
        match name.trim() {
            "realm" => realm = Some(value),
            "nonce" => nonce = Some(value),
            "qop" => qop = Some(value),
            "opaque" => opaque = Some(value),
            _ => {}
        }
        rest = after.trim_start().trim_start_matches(',').trim_start();
    }
    Some(Challenge { realm: realm?, nonce: nonce?, qop, opaque })
}

fn md5_hex(text: &str) -> String {
    Md5::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The Authorization header answering challenge, as marco-sparko's src/myenergi/digest.rs.
fn digest_authorization(challenge: &Challenge, username: &str, password: &str, method: &str, uri: &str, nc: u32, cnonce: &str) -> String {
    let ha1 = md5_hex(&format!("{username}:{}:{password}", challenge.realm));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    // Only auth is supported, which is all that servers which offer a choice are known to need.
    let qop = challenge.qop.as_deref().map(|qop| qop.split(',').map(str::trim).find(|qop| *qop == "auth").unwrap_or("auth"));
    let mut header =
        format!("Digest username=\"{username}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\"", challenge.realm, challenge.nonce);
    match qop {
        Some(qop) => {
            let response = md5_hex(&format!("{ha1}:{}:{nc:08x}:{cnonce}:{qop}:{ha2}", challenge.nonce));
            header.push_str(&format!(", qop={qop}, nc={nc:08x}, cnonce=\"{cnonce}\", response=\"{response}\""));
        }
        None => {
            let response = md5_hex(&format!("{ha1}:{}:{ha2}", challenge.nonce));
            header.push_str(&format!(", response=\"{response}\""));
        }
    }
    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(", opaque=\"{opaque}\""));
    }
    header.push_str(", algorithm=MD5");
    header
}

/// The status with only what the app uses, see ZAPPI_FIELDS, in the same form, an array of objects such as
/// {"zappi": [{"sno": 12345678, ...}]}.
fn reduce(status: &Value) -> Value {
    let items: Vec<Value> = status
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_object())
        .flat_map(|item| item.iter())
        .filter(|(kind, _)| DEVICE_KINDS.contains(&kind.as_str()))
        .map(|(kind, devices)| {
            let devices: Vec<Value> = devices
                .as_array()
                .into_iter()
                .flatten()
                .map(|device| {
                    let fields: &[&str] = if kind == "zappi" { ZAPPI_FIELDS } else { &["sno"] };
                    let kept: serde_json::Map<String, Value> = fields
                        .iter()
                        .filter_map(|field| Some((field.to_string(), device.get(*field)?.clone())))
                        .collect();
                    Value::Object(kept)
                })
                .collect();
            json!({ kind.as_str(): devices })
        })
        .collect();
    Value::Array(items)
}

/// What the first zappi in a status is doing, for the screen, e.g. "zappi 3.4 kWh 7.1 kW".
fn panel_line(status: &Value) -> Line {
    let zappi = status
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|item| item.get("zappi")?.as_array()?.first().cloned());
    let Some(zappi) = zappi else {
        return ("myenergi: no zappi".to_string(), false);
    };
    let pst = zappi.get("pst").and_then(|value| value.as_str()).unwrap_or_default();
    let che = zappi.get("che").and_then(|value| value.as_f64()).unwrap_or_default();
    let kw = zappi.get("div").and_then(|value| value.as_f64()).unwrap_or_default() / 1000.0;
    // The plug's state, as marco-sparko's plug_state, shortened.
    let state = match pst {
        "A" => return ("zappi: no car".to_string(), false),
        "B1" => "connected",
        "B2" => "waiting",
        "C1" => "ready",
        "C2" => "charging",
        "F" => return ("zappi: fault".to_string(), true),
        other => other,
    };
    if kw > 0.0 {
        (format!("zappi {che:.1} kWh {kw:.1} kW"), false)
    } else {
        (format!("zappi {che:.1} kWh {state}"), false)
    }
}

struct PollTask {
    myenergi: Myenergi,
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
        "myenergi Capture"
    }
}

impl PollTask {
    /// Polls the status, saving the record, or showing the error.
    fn poll(&mut self, slot: Option<DateTime<Utc>>) {
        self.kept.release();
        let requested = Utc::now();
        let result = self.myenergi.get("/cgi-jstatus-*");
        let received = Utc::now();
        let heap = unsafe { esp_idf_sys::esp_get_free_heap_size() };
        let minimum_heap = unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() };
        let mut store = self.store.lock().unwrap();
        match result {
            Ok(status) => {
                let response = reduce(&status);
                store.line = panel_line(&response);
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
                    "response": response,
                })
                .to_string();
                let time = slot.unwrap_or(requested);
                store.files.append(time, &record);
                let Store { files, ram, .. } = &mut *store;
                files.keep(ram, RAM_CAPACITY, time, record);
                info!("myenergi poll saved, free heap {heap}, minimum {minimum_heap}");
            }
            Err(error) => {
                warn!("myenergi poll failed, {error:#}");
                store.line = (format!("myenergi: {error:#}"), true);
            }
        }
        self.panel.set(PANEL_ORDER, vec![store.line.clone()]);
    }
}
