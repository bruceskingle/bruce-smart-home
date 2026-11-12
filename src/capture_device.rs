//! Capture devices: ESP32s, see capture/ in this repo, which poll what the modules need while the computer sleeps, and
//! which the app manages through their HTTPS API, see bruce-esp-idf's api.rs.
//!
//! A device's certificate is one it made itself, so the app pins it: it notes the certificate's fingerprint the first
//! time it connects, checks it against what the device says in /api/info, and from then on only talks to a device with
//! that certificate, see PinnedCertificate. Adopting a device, see adopt, gives it a key the app made, which every
//! request but /api/info then needs.
//!
//! Reading, e.g. /api/info for the Devices page, and a device's records, is over plain HTTP, as HTTPS needs tens of KB
//! of a device's RAM, which it may not have while it polls over HTTPS itself. Reads of records are signed with the key,
//! see sign, so that it is never sent where it could be seen. Changes, e.g. of the config, which has passwords in it,
//! and adoption, which sends the key, are over HTTPS.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// A device the profile has adopted, in its settings, see profile::update_capture_devices.
#[derive(Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProfile {
    /// The device's ID, its Wi-Fi MAC address, which never changes.
    pub id: String,
    /// Its IP address or host name, which may change.
    pub address: String,
    /// The SHA-256 of its certificate, as lower case hex, see PinnedCertificate.
    pub fingerprint: String,
    /// The key the app gave it when it adopted it, 32 bytes as hex.
    pub key: String,
}

/// Debug output is used in verbose mode, so the key is hidden.
impl std::fmt::Debug for DeviceProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceProfile")
            .field("id", &self.id)
            .field("address", &self.address)
            .field("fingerprint", &self.fingerprint)
            .field("key", &bruce_graphql::redact::REDACTED)
            .finish()
    }
}

/// What a device says it is, from /api/info.
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    /// Its name on the network, e.g. cyd, if it has one.
    pub hostname: Option<String>,
    pub board: String,
    /// The chip it is running on, as it says it is, whichever board its firmware was built for. Devices from before
    /// it was given say nothing.
    #[serde(default)]
    pub chip: Option<Chip>,
    pub firmware: Software,
    pub platform: Software,
    pub certificate_fingerprint: String,
    pub adopted: bool,
    pub up_time_seconds: u64,
    /// Why it last started, e.g. Panic, see started_after. Devices from before it was given say nothing.
    #[serde(default)]
    pub reset_reason: Option<String>,
    /// Why it crashed before this start, if it did and knows: a panic's message, or that it ran out of memory.
    #[serde(default)]
    pub last_crash: Option<String>,
    pub features: Vec<FeatureState>,
}

/// A device's chip, see DeviceInfo::chip.
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chip {
    /// e.g. ESP32-C6.
    pub model: String,
    /// e.g. v0.2.
    pub revision: String,
    pub cores: u8,
    pub flash_bytes: u64,
    pub psram_bytes: u64,
}

impl Chip {
    /// e.g. ESP32-C6 v0.2, 1 core, 8 MB flash, no PSRAM, as the device's /config page says it.
    pub fn describe(&self) -> String {
        let mb = |bytes: u64| format!("{} MB", bytes / (1024 * 1024));
        let cores = if self.cores == 1 { "1 core".to_string() } else { format!("{} cores", self.cores) };
        let psram = if self.psram_bytes == 0 { "no PSRAM".to_string() } else { format!("{} PSRAM", mb(self.psram_bytes)) };
        format!("{} {}, {cores}, {} flash, {psram}", self.model, self.revision, mb(self.flash_bytes))
    }
}

#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct Software {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct FeatureState {
    pub name: String,
    /// What to show it as, e.g. Tesla Powerwall, if the device says.
    pub label: Option<String>,
    pub enabled: bool,
}

/// When and why a device started, from /api/starts.
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct Start {
    /// RFC 3339.
    pub time: String,
    /// e.g. Panic, see DeviceInfo::reset_reason.
    pub reason: String,
    /// Its number, counted from the device's first start, None if the device could not count it.
    pub count: Option<u32>,
}

/// A device's latest starts, oldest first, from the log it keeps in its NVS, or, with older firmware, on its SD
/// card, or only its last if it could not read the log, e.g. it has no card.
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct Starts {
    /// The last start's number, see Start::count.
    pub count: Option<u32>,
    pub starts: Vec<Start>,
    /// Why the device could only say when it last started, e.g. older firmware with no SD card.
    pub problem: Option<String>,
}

/// The device responded with a status other than success, with the reason it gave, if any, e.g. 404 for a feature it
/// does not have.
#[derive(Debug)]
pub struct StatusError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.status)
    }
}

impl std::error::Error for StatusError {}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Accepts only the certificate with the expected fingerprint, or, the first time, when there is none yet, any, noting
/// its fingerprint in seen, to check against what the device says and pin from then on.
#[derive(Debug)]
struct PinnedCertificate {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>, _ocsp_response: &[u8], _now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let fingerprint = sha256_hex(end_entity.as_ref());
        *self.seen.lock().unwrap() = Some(fingerprint.clone());
        match &self.expected {
            Some(expected) if *expected != fingerprint => Err(rustls::Error::General(format!(
                "the device's certificate is not the one it had when it was adopted ({fingerprint} instead of {expected}), \
                 it may have been reset, or something else may be at its address"))),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    // The certificate's signature still proves the device has its key.
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// How long to wait to connect to a device over plain HTTP, see DeviceClient::info, short, as a device on the network
/// answers at once, so that one which is not, e.g. while it is being flashed, is soon shown as not.
const PLAIN_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long /api/info may take, longer than connecting, as the device may first finish sending a page of records.
const INFO_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a page of records may take, see CaptureReader::read_page.
const PAGE_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// The Authorization header of a request, of method, for uri, its path and query as sent, signed with key, the 32 bytes
/// of a device key as hex, at time, in seconds since 1970, as a device checks it, see bruce-esp-idf's
/// Adoption::is_signed.
fn sign(key: &str, method: &str, uri: &str, time: i64) -> anyhow::Result<String> {
    use hmac::Mac;

    let key = (0..key.len()).step_by(2)
        .map(|at| key.get(at..at + 2).and_then(|byte| u8::from_str_radix(byte, 16).ok()))
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| anyhow!("the device key is not hex"))?;
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(&key)?;
    mac.update(format!("{method} {uri} {time}").as_bytes());
    let signature: String = mac.finalize().into_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("Sparko-HMAC-SHA256 {time}:{signature}"))
}

/// The addresses of the devices which have taken a signed request over plain HTTP, see get_query.
static SIGNED: Mutex<std::collections::BTreeSet<String>> = Mutex::new(std::collections::BTreeSet::new());

/// A connection to one device's API.
pub struct DeviceClient {
    http: reqwest::Client,
    base: String,
    /// For plain HTTP, see the top of this file.
    plain: reqwest::Client,
    address: String,
    key: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl DeviceClient {
    /// A client for the device at address, which only accepts the certificate with fingerprint, if given, and sends
    /// key, if given.
    pub fn new(address: &str, fingerprint: Option<&str>, key: Option<&str>) -> anyhow::Result<DeviceClient> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let seen = Arc::new(Mutex::new(None));
        let verifier = PinnedCertificate { expected: fingerprint.map(String::from), seen: seen.clone(), provider: provider.clone() };
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        let http = crate::http::client_builder()
            .use_preconfigured_tls(tls)
            // Long enough for a device which is busy with its own poll, see CaptureReader::read_device.
            .connect_timeout(Duration::from_secs(15))
            // Long enough for a night's 10 second samples, a few MB, which a device sends slowly.
            .timeout(Duration::from_secs(5 * 60))
            .build()?;
        let plain = crate::http::client_builder().connect_timeout(PLAIN_CONNECT_TIMEOUT).build()?;
        Ok(DeviceClient {
            http,
            base: format!("https://{}/api", address.trim()),
            plain,
            address: address.trim().to_string(),
            key: key.map(String::from),
            seen,
        })
    }

    /// For a device the profile has adopted.
    pub fn for_device(device: &DeviceProfile) -> anyhow::Result<DeviceClient> {
        DeviceClient::new(&device.address, Some(&device.fingerprint), Some(&device.key))
    }

    /// The fingerprint of the certificate the device presented, once connected.
    pub fn seen_fingerprint(&self) -> Option<String> {
        self.seen.lock().unwrap().clone()
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let lock = device_lock(&self.base);
        let _turn = lock.lock().await;
        let request = match &self.key {
            Some(key) => request.bearer_auth(key),
            None => request,
        };
        Self::receive(request).await
    }

    /// Makes request, returning the JSON the device responded with, or an error, a StatusError if it said no.
    async fn receive(request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            // The API's errors are JSON, but not the HTTP server's own, e.g. 404 for a path no feature has.
            let message = serde_json::from_str::<Value>(&text).ok()
                .and_then(|json| json.get("error").and_then(|error| error.as_str()).map(String::from))
                .unwrap_or_else(|| crate::http::summary(&text));
            return Err(StatusError { status: status.as_u16(), message }.into());
        }
        serde_json::from_str(&text).map_err(|_| anyhow!("the device responded {status}: {}", crate::http::summary(&text)))
    }

    pub async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.send(self.http.get(format!("{}/{path}", self.base))).await
    }

    /// Reads path, with query, over plain HTTP, signed with the key, see sign, or over HTTPS if there is no key, or
    /// the device does not take signed requests, e.g. its firmware is from before them, or its clock is wrong.
    pub async fn get_query(&self, path: &str, query: &[(&str, String)]) -> anyhow::Result<Value> {
        self.get_signed(path, query, true).await
    }

    /// As get_query, but if missing_over_https is false, a path the device does not have, 404, is not asked for again
    /// over HTTPS, e.g. /api/store/info, which a device from before the store does not have, whether or not it takes
    /// signed requests.
    async fn get_signed(&self, path: &str, query: &[(&str, String)], missing_over_https: bool) -> anyhow::Result<Value> {
        if let Some(key) = &self.key {
            let mut url = reqwest::Url::parse(&format!("http://{}/api/{path}", self.address))?;
            if !query.is_empty() {
                url.query_pairs_mut().extend_pairs(query);
            }
            let uri = match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_string(),
            };
            // One request at a time to each device, as over HTTPS, see send: a device answering requests which overlap
            // gives wrong answers, e.g. no records, or another feature's, seen on the RLCD on 5 November 2026.
            let lock = device_lock(&self.base);
            let _turn = lock.lock().await;
            let authorization = sign(key, "GET", &uri, time::OffsetDateTime::now_utc().unix_timestamp())?;
            let request = self.plain.get(url).header(reqwest::header::AUTHORIZATION, authorization).timeout(PAGE_TIMEOUT);
            let result = Self::receive(request).await;
            drop(_turn);
            // A 404 from a device which has taken a signed request is a path it does not have, e.g. /api/store/info
            // on firmware from before the store, not firmware from before signed requests, so not asked again over
            // HTTPS, which needs tens of KB of its RAM.
            let signed = !missing_over_https || SIGNED.lock().unwrap().contains(&self.address);
            match result {
                Err(error) if error.downcast_ref::<StatusError>().is_some_and(|error| error.status == 401 || (error.status == 404 && !signed)) => {
                    crate::log_info!("Capture device at {} did not take a signed request for {path}, using HTTPS, {error:#}", self.address);
                }
                result => {
                    if result.is_ok() {
                        SIGNED.lock().unwrap().insert(self.address.clone());
                    }
                    return result;
                }
            }
        }
        self.send(self.http.get(format!("{}/{path}", self.base)).query(query)).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        self.send(self.http.post(format!("{}/{path}", self.base)).json(body)).await
    }

    /// What the device says it is, over plain HTTP, to be quick, see PLAIN_CONNECT_TIMEOUT, and to not need the RAM
    /// HTTPS does, as it is public, and a device which is not the one adopted does not get the key, see secure_info.
    pub async fn info(&self) -> anyhow::Result<DeviceInfo> {
        let request = self.plain.get(format!("http://{}/api/info", self.address)).timeout(INFO_TIMEOUT);
        Ok(serde_json::from_value(Self::receive(request).await?)?)
    }

    /// What the device says it is, over HTTPS, so with the certificate it presented, see seen_fingerprint, e.g. to pin
    /// when adopting it.
    pub async fn secure_info(&self) -> anyhow::Result<DeviceInfo> {
        Ok(serde_json::from_value(self.get("info").await?)?)
    }

    /// Asks the device to show a pairing code, returning how long it lasts.
    pub async fn start_pairing(&self) -> anyhow::Result<Duration> {
        let json = self.post("adopt/start", &serde_json::json!({})).await?;
        Ok(Duration::from_secs(json["expiresInSeconds"].as_u64().unwrap_or_default()))
    }

    pub async fn release(&self) -> anyhow::Result<()> {
        self.post("release", &serde_json::json!({})).await.map(|_| ())
    }

    /// When and why the device started, see Starts, signed over plain HTTP, see get_query.
    pub async fn starts(&self) -> anyhow::Result<Starts> {
        Ok(serde_json::from_value(self.get_query("starts", &[]).await?)?)
    }

    pub async fn restart(&self) -> anyhow::Result<()> {
        self.post("restart", &serde_json::json!({})).await.map(|_| ())
    }

    /// Every feature's config, see bruce-embedded-std's config_json.rs.
    pub async fn config(&self) -> anyhow::Result<Value> {
        self.get("config").await
    }

    /// Changes the config, which takes effect when the device restarts, returning the new config.
    pub async fn update_config(&self, update: &Value) -> anyhow::Result<Value> {
        let json = self.post("config", update).await?;
        Ok(json["config"].clone())
    }
}

/// The devices the profile has adopted, as saved now, not when the app started, as the Devices page may have adopted
/// or released one since.
pub fn adopted_devices(profile_name: &str) -> Vec<DeviceProfile> {
    crate::profile::fetch_active_profile(&Some(profile_name.to_string()))
        .map(|profile| profile.active_profile.settings.capture_devices)
        .unwrap_or_default()
}

/// A device found at an address, not yet adopted, with what it says it is and the certificate it presented.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundDevice {
    pub address: String,
    pub info: DeviceInfo,
    pub fingerprint: String,
}

/// Asks the device at address what it is, checking that the certificate it presented is the one it says it has, so
/// that it can be pinned.
pub async fn find(address: &str) -> anyhow::Result<FoundDevice> {
    let address = address.trim();
    let client = DeviceClient::new(address, None, None)?;
    let info = client.secure_info().await.map_err(|error| anyhow!("no capture device answered at {address}, {error:#}"))?;
    let fingerprint = client.seen_fingerprint().ok_or_else(|| anyhow!("the device presented no certificate"))?;
    if fingerprint != info.certificate_fingerprint {
        return Err(anyhow!("the certificate presented at {address} is not the one the device there says it has, \
            something else may be in between"));
    }
    Ok(FoundDevice { address: address.to_string(), info, fingerprint })
}

/// Asks a found device to show its pairing code, see adopt.
pub async fn start_pairing(found: &FoundDevice) -> anyhow::Result<Duration> {
    DeviceClient::new(&found.address, Some(&found.fingerprint), None)?.start_pairing().await
}

/// Adopts a found device with the code on its screen, giving it a new key, and returns it for the profile.
pub async fn adopt(found: &FoundDevice, code: &str) -> anyhow::Result<DeviceProfile> {
    let key: String = rand::random::<[u8; 32]>().iter().map(|byte| format!("{byte:02x}")).collect();
    let client = DeviceClient::new(&found.address, Some(&found.fingerprint), None)?;
    client.post("adopt", &serde_json::json!({ "code": code.trim(), "key": key })).await?;
    crate::log_info!("Adopted the capture device {} at {}", found.info.id, found.address);
    Ok(DeviceProfile { id: found.info.id.clone(), address: found.address.clone(), fingerprint: found.fingerprint.clone(), key })
}

/// How long to wait before asking a device which did not answer again, see CaptureReader::read_device.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// A lock for each device, by its address, so that every request to it, e.g. from the modules which read it, which the
/// app polls together, see polling.rs, and from the Devices page, is made one at a time, as a device only has room for
/// one or two HTTPS connections at once.
fn device_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        std::sync::LazyLock::new(Default::default);
    LOCKS.lock().unwrap().entry(id.to_string()).or_default().clone()
}

/// How long after a device's poll the app reads its records, when the device has finished polling, see
/// CaptureReader::next_read.
const READ_DELAY: Duration = Duration::from_secs(60);

/// How many records are asked for at once, see CaptureReader::read_device, so that catching up, e.g. after the computer
/// slept, is a number of short requests, each saved before the next, rather than one which takes minutes.
const PAGE_SIZE: usize = 500;

/// The first time of a cron spec, with seconds first, e.g. 0 */5 * * * *, after after.
fn next_occurrence(spec: &str, after: time::OffsetDateTime) -> Option<time::OffsetDateTime> {
    use std::str::FromStr;

    let cron = croner::Cron::from_str(spec).ok()?;
    let after = chrono::DateTime::from_timestamp(after.unix_timestamp(), after.nanosecond())?;
    let next = cron.find_next_occurrence(&after, false).ok()?;
    time::OffsetDateTime::from_unix_timestamp(next.timestamp()).ok()
}

/// Reads one kind of record, e.g. the Powerwall's, from the profile's capture devices which take it, see read, so that
/// the app has them for the times it was not running, e.g. while the computer slept.
pub struct CaptureReader {
    /// After /api/, e.g. capture/tesla/records.
    path: &'static str,
    /// The module whose records they are, e.g. tesla, the part of path after capture/.
    module_id: String,
    /// The time of the latest record read from each device, by its ID, to only ask for later ones. Not kept between
    /// runs: the first read of a device asks for those after the default given to read.
    read_until: std::sync::Mutex<std::collections::HashMap<String, time::OffsetDateTime>>,
    /// When each device which takes the records polls, by its ID, as it said with them, see next_read.
    schedules: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

/// What a device says with its records, e.g. when and why it last started, for the log and the Polling page, see
/// DeviceStatus.
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordsDevice {
    pub boot_time: Option<String>,
    pub reset_reason: Option<String>,
    /// Where it keeps its records, e.g. sd, or ram, see storage_problem.
    pub storage: Option<String>,
    /// Why it keeps its records in RAM, e.g. ESP_ERR_TIMEOUT, as it has no SD card.
    pub storage_problem: Option<String>,
    /// The SD card's size and free space, when it keeps its records on one. Devices from before it was given do not
    /// say.
    pub sd_card: Option<SdCard>,
    pub free_heap: Option<u64>,
    /// The least free heap since it started, which shows how near it has come to running out.
    pub minimum_free_heap: Option<u64>,
    /// How many records it has in RAM, and the most it keeps there, when it has no SD card.
    pub records_in_ram: Option<u64>,
    pub ram_capacity: Option<u64>,
    /// The largest block of the free heap, which an internet connection needs some KB of, and how many allocations
    /// have failed since it started.
    pub largest_free_block: Option<u64>,
    pub failed_allocations: Option<u64>,
    /// Why it crashed before this start, see DeviceInfo::last_crash.
    pub last_crash: Option<String>,
    /// How its latest poll of these records' source went, e.g. that Octopus did not accept the API key. Devices from
    /// before it was given do not say.
    pub last_poll: Option<LastPoll>,
    /// When it polls, a cron spec, see CaptureReader::next_read.
    schedule: Option<String>,
}

/// A device's SD card, see RecordsDevice::sd_card.
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SdCard {
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// How a device's latest poll of a source went, as its screen shows it: when, the error if it failed, and when a poll
/// last succeeded, the times as RFC 3339.
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastPoll {
    pub time: String,
    pub error: Option<String>,
    pub last_success: Option<String>,
}

/// How reading a profile's capture devices is going, for the Polling page, see device_statuses.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DeviceStatus {
    pub id: String,
    pub address: String,
    /// Its name on the network, e.g. cyd, see DeviceInfo::hostname, once it has been asked.
    pub name: Option<String>,
    /// The modules whose records it takes, by module id, e.g. tesla, see CaptureReader::path.
    pub takes: std::collections::BTreeSet<String>,
    /// When it was last read, and when last asked, with why that failed, if it did.
    pub last_read: Option<time::OffsetDateTime>,
    pub last_error: Option<(time::OffsetDateTime, String)>,
    /// The records read from it since the app started, and how many of them the app did not have already.
    pub read: u64,
    pub saved: u64,
    /// What it said with its records last.
    pub device: Option<RecordsDevice>,
    /// How its latest poll of each source went, by module id, e.g. octopus, see RecordsDevice::last_poll.
    pub polls: std::collections::BTreeMap<String, LastPoll>,
}

/// The status of each of the profile's capture devices, by ID, since the poller started, see clear_statuses.
static STATUSES: Mutex<std::collections::BTreeMap<String, DeviceStatus>> = Mutex::new(std::collections::BTreeMap::new());

/// The status of each capture device read since the poller started, by ID. The Polling page shows them after each
/// poll, which is when they change.
pub fn device_statuses() -> Vec<DeviceStatus> {
    STATUSES.lock().unwrap().values().cloned().collect()
}

/// Forgets the statuses, when the poller starts, e.g. for another profile.
pub fn clear_statuses() {
    STATUSES.lock().unwrap().clear();
}

/// The names of the devices which take the module's records, e.g. ["WaveTouch", "cyd"], by ID if they have no name.
pub fn devices_taking(module_id: &str) -> Vec<String> {
    STATUSES.lock().unwrap().values()
        .filter(|status| status.takes.contains(module_id))
        .map(|status| status.name.clone().unwrap_or_else(|| status.id.clone()))
        .collect()
}

/// Changes the status of device with change, adding it if it is new.
fn update_status(device: &DeviceProfile, change: impl FnOnce(&mut DeviceStatus)) {
    let mut statuses = STATUSES.lock().unwrap();
    let status = statuses.entry(device.id.clone()).or_insert_with(|| DeviceStatus { id: device.id.clone(), ..Default::default() });
    status.address = device.address.clone();
    change(status);
}

/// Logs a device's poll of the module starting to fail, or recovering, from previous to poll, so that the log says when
/// it happened, without an entry for every read while it goes on.
fn log_poll_change(status: &DeviceStatus, module_id: &str, previous: Option<&LastPoll>, poll: &LastPoll) {
    let name = status.name.as_deref().unwrap_or(&status.id);
    match (previous.and_then(|previous| previous.error.as_ref()), &poll.error) {
        (None, Some(error)) => crate::log_warning!("The capture device {name} is unable to poll {module_id}, {error}"),
        (Some(_), None) => crate::log_info!("The capture device {name} polled {module_id} again"),
        _ => {}
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Records {
    device: RecordsDevice,
    records: Vec<Value>,
    /// Why the device could not read all of them, after which it gave those in RAM.
    records_error: Option<String>,
    /// There are more than were asked for, which come next, since the last of these. Devices from before paging
    /// give them all, and never say so.
    #[serde(default)]
    more: bool,
}

/// When a record is of: its slot, if it was taken on one, otherwise when it was requested.
pub fn record_time(record: &Value) -> Option<time::OffsetDateTime> {
    ["slot", "requested", "end"].iter()
        .find_map(|name| record.get(*name)?.as_str())
        .and_then(|time| time::OffsetDateTime::parse(time, &time::format_description::well_known::Rfc3339).ok())
}

impl CaptureReader {
    pub fn new(path: &'static str) -> CaptureReader {
        CaptureReader {
            path,
            module_id: path.split('/').nth(1).unwrap_or(path).to_string(),
            read_until: std::sync::Mutex::new(std::collections::HashMap::new()),
            schedules: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// When to read next: READ_DELAY after the first poll of any device which takes the records at or after the app's
    /// next slot of interval, see polling::next_slot, so that the app reads as often as it would poll, however often a
    /// device polls, e.g. every 10 seconds, and not while a device is busy polling. None if no device has said when it
    /// polls, see Module::next_poll.
    pub fn next_read(&self, now: time::OffsetDateTime, interval: Duration) -> Option<time::OffsetDateTime> {
        let slot = crate::polling::next_slot(now, interval);
        self.schedules.lock().unwrap().values()
            .filter_map(|spec| next_occurrence(spec, slot - Duration::from_secs(1)))
            .min()
            .map(|next| next + READ_DELAY)
    }

    /// Reads the records of each device which takes them: a device on the store is replicated into store, see
    /// replicate, and one with older firmware is read those since since, the time of the latest the app has, the first
    /// time, and since the latest read after that, saving each with save, given the device it is from, which says
    /// whether it was new.
    /// Returns false if none of them takes them, e.g. there are none, so that the app fetches them itself, and an error
    /// if none which does could be read.
    pub async fn read(&self, store: &bruce_db::Store, devices: &[DeviceProfile], since: Option<time::OffsetDateTime>,
        save: &mut dyn FnMut(&DeviceProfile, &Value) -> anyhow::Result<bool>, progress: &mut dyn FnMut(String)) -> anyhow::Result<bool> {
        self.read_each(store, devices, &|_| since, save, progress).await
    }

    /// As read, with since_for giving each device's since, e.g. the latest the app has of that device's own records,
    /// for records which are each device's own, e.g. its temperature, rather than copies of a source's.
    pub async fn read_each(&self, store: &bruce_db::Store, devices: &[DeviceProfile],
        since_for: &dyn Fn(&DeviceProfile) -> Option<time::OffsetDateTime>,
        save: &mut dyn FnMut(&DeviceProfile, &Value) -> anyhow::Result<bool>, progress: &mut dyn FnMut(String)) -> anyhow::Result<bool> {
        let mut takes = false;
        let mut read_any = false;
        let mut first_error = None;
        for device in devices {
            // A device on the store, replicated, rather than read, see replicate. Older firmware has no
            // /api/store/info, so is read as before.
            let result = match replicate(store, device, progress).await {
                Ok(Some(info)) => match info.features.get(&self.module_id) {
                    Some(feature) => {
                        if let Some(schedule) = &feature.schedule {
                            self.schedules.lock().unwrap().insert(device.id.clone(), schedule.clone());
                        }
                        Ok(())
                    }
                    None => Err(StatusError { status: 404, message: format!("It does not take {}", self.module_id) }.into()),
                },
                Ok(None) => self.read_device(device, since_for(device), save, progress).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => {
                    takes = true;
                    read_any = true;
                }
                // It does not take them.
                Err(error) if error.downcast_ref::<StatusError>().is_some_and(|error| error.status == 404) => {
                    self.schedules.lock().unwrap().remove(&device.id);
                    update_status(device, |status| { status.takes.remove(&self.module_id); });
                }
                Err(error) => {
                    takes = true;
                    crate::log_warning!("Unable to read {} from the capture device {}, {error:#}", self.path, device.id);
                    update_status(device, |status| status.last_error = Some((crate::clock::now_utc(), format!("{error:#}"))));
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) if !read_any => Err(error),
            _ => Ok(takes),
        }
    }

    /// Reads the records of device a page at a time, see PAGE_SIZE, saving each page before asking for the next.
    async fn read_device(&self, device: &DeviceProfile, since: Option<time::OffsetDateTime>,
        save: &mut dyn FnMut(&DeviceProfile, &Value) -> anyhow::Result<bool>, progress: &mut dyn FnMut(String)) -> anyhow::Result<()> {
        let mut since = self.read_until.lock().unwrap().get(&device.id).copied().or(since);
        let client = DeviceClient::for_device(device)?;
        // Its name, once, for the Polling page.
        let named = STATUSES.lock().unwrap().get(&device.id).is_some_and(|status| status.name.is_some());
        if !named {
            match client.info().await {
                Ok(info) => update_status(device, |status| status.name = info.hostname.filter(|name| !name.is_empty())),
                Err(error) => crate::log_info!("Unable to get the name of the capture device {}, {error:#}", device.id),
            }
        }
        let mut read = 0;
        let mut saved = 0;
        progress(format!("Reading the capture device {}", device.id));
        loop {
            let records = self.read_page(&client, device, since).await?;
            read += records.records.len();
            let page_saved = saved;
            let mut latest = since;
            for record in &records.records {
                let Some(time) = record_time(record) else {
                    crate::log_warning!("A capture device record has no time, {}", crate::http::summary(&record.to_string()));
                    continue;
                };
                if save(device, record)? {
                    saved += 1;
                }
                latest = latest.max(Some(time));
            }
            if let Some(latest) = latest {
                self.read_until.lock().unwrap().insert(device.id.clone(), latest);
            }
            update_status(device, |status| {
                status.takes.insert(self.module_id.clone());
                status.last_read = Some(crate::clock::now_utc());
                status.last_error = None;
                status.read += records.records.len() as u64;
                status.saved += (saved - page_saved) as u64;
                status.device = Some(records.device.clone());
                if let Some(poll) = &records.device.last_poll {
                    log_poll_change(status, &self.module_id, status.polls.get(&self.module_id), poll);
                    status.polls.insert(self.module_id.clone(), poll.clone());
                }
            });
            // Only while the page moved on, so that a device which said more of nothing is not asked forever.
            if !records.more || latest == since {
                break;
            }
            since = latest;
            progress(format!("Read {read} samples from the capture device {} so far, saved {saved}", device.id));
        }
        progress(format!("Saved {saved} of the {read} samples from the capture device {}", device.id));
        Ok(())
    }

    /// Reads a page of the records of device, those after since.
    async fn read_page(&self, client: &DeviceClient, device: &DeviceProfile, since: Option<time::OffsetDateTime>)
        -> anyhow::Result<Records> {
        let rfc3339 = &time::format_description::well_known::Rfc3339;
        let mut query: Vec<(&str, String)> = since.map(|since| since.format(rfc3339)).transpose()?
            .into_iter().map(|since| ("since", since)).collect();
        query.push(("limit", PAGE_SIZE.to_string()));
        crate::log_info!("Capture device {} GET /api/{} {query:?}", device.id, self.path);
        let json = match client.get_query(self.path, &query).await {
            // The app polls on the same slots as the device, which may still be busy with its own polls, e.g. of
            // myenergi over the internet, so once more when it should have finished.
            Err(error) if error.downcast_ref::<reqwest::Error>().is_some_and(|error| error.is_connect() || error.is_timeout()) => {
                crate::log_info!("Capture device {} did not answer, trying again in {} s, {error:#}", device.id, RETRY_AFTER.as_secs());
                tokio::time::sleep(RETRY_AFTER).await;
                client.get_query(self.path, &query).await?
            }
            result => result?,
        };
        let records: Records = serde_json::from_value(json)?;
        let info = &records.device;
        if let Some(schedule) = &info.schedule {
            self.schedules.lock().unwrap().insert(device.id.clone(), schedule.clone());
        }
        crate::log_info!("Capture device {}: {} records{}, kept on {:?} {:?}, started {:?} after {:?}, free heap {:?}",
            device.id, records.records.len(), if records.more { " and more" } else { "" }, info.storage,
            info.storage_problem, info.boot_time, info.reset_reason, info.free_heap);
        if let Some(error) = &records.records_error {
            crate::log_warning!("Capture device {} could not read all its records, {error}", device.id);
        }
        Ok(records)
    }
}

/// How many changes a device is asked for at a time, see replicate. A page of 500 takes a Guitron some 14 seconds.
const CHANGES_PAGE: usize = 500;
/// How long a device's replication is taken as current, so that the modules it takes, polled together on the same
/// slot, do not each pull its changes, see replicate.
const REPLICATED_FOR: Duration = Duration::from_secs(5);
/// The kind of the app's cursor of each device's change feed, see replicate.
const REPLICATION_KIND: &str = "app.replication";

/// What a device on the store says of it, /api/store/info: the site and the store's ID, and each feature's status, by
/// the module whose records it takes, e.g. tesla, which its records pages said before the store.
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StoreInfo {
    pub site: Option<String>,
    pub store: Option<String>,
    /// Why it has no store, e.g. no SD card, or the site not set.
    pub problem: Option<String>,
    #[serde(default)]
    pub features: std::collections::BTreeMap<String, RecordsDevice>,
}

/// A page of a device's change feed, /api/store/changes.
#[derive(Deserialize)]
struct ChangesPage {
    store: String,
    changes: Vec<Value>,
    next: u64,
    /// Why it could not read them all, after which it gave those it had.
    error: Option<String>,
}

/// The latest replication of each device, by its ID, with what it said of itself then, see REPLICATED_FOR.
static REPLICATIONS: Mutex<std::collections::BTreeMap<String, Arc<tokio::sync::Mutex<Option<(std::time::Instant, StoreInfo)>>>>> =
    Mutex::new(std::collections::BTreeMap::new());

/// Pulls the changes a device on the store has made since the app last did, putting each into store as from the
/// device, see the design's capture devices on the store, and returns what the device says of itself, or None if its
/// firmware is from before the store, so that it is read as it was. The cursor is the app's, in store, with the
/// device's store's ID, so that a device whose store is new, e.g. a changed card, is pulled from the start again.
pub async fn replicate(store: &bruce_db::Store, device: &DeviceProfile, progress: &mut dyn FnMut(String))
    -> anyhow::Result<Option<StoreInfo>> {
    let latest = REPLICATIONS.lock().unwrap().entry(device.id.clone()).or_default().clone();
    let mut latest = latest.lock().await;
    if let Some((at, info)) = latest.as_ref().filter(|(at, _)| at.elapsed() < REPLICATED_FOR) {
        crate::log_info!("Capture device {} replicated {} ms ago", device.id, at.elapsed().as_millis());
        return Ok(Some(info.clone()));
    }
    let client = DeviceClient::for_device(device)?;
    let named = STATUSES.lock().unwrap().get(&device.id).is_some_and(|status| status.name.is_some());
    if !named {
        match client.info().await {
            Ok(info) => update_status(device, |status| status.name = info.hostname.filter(|name| !name.is_empty())),
            Err(error) => crate::log_info!("Unable to get the name of the capture device {}, {error:#}", device.id),
        }
    }
    let info: StoreInfo = match client.get_signed("store/info", &[], false).await {
        Ok(json) => serde_json::from_value(json)?,
        Err(error) if error.downcast_ref::<StatusError>().is_some_and(|error| error.status == 404) => return Ok(None),
        Err(error) => return Err(error),
    };
    if let Some(problem) = &info.problem {
        anyhow::bail!("The capture device {} keeps no store, {problem}", device.id);
    }
    let site = store.site().to_string();
    if info.site.as_deref() != Some(site.as_str()) {
        anyhow::bail!("The capture device {} keeps the site {}, not this profile's", device.id, info.site.as_deref().unwrap_or("none"));
    }
    let hk = store.hash_key(REPLICATION_KIND, &[bruce_db::Part::Name(crate::store::node()?),
        bruce_db::Part::Name(crate::store::device_part(&device.id))])?;
    let cursor = store.get(&hk, "-")?.and_then(|item| item.attributes);
    let mut since = cursor.as_ref()
        .filter(|cursor| cursor["store"].as_str() == info.store.as_deref())
        .and_then(|cursor| cursor["next"].as_u64())
        .unwrap_or(0);
    progress(format!("Reading the capture device {}", device.id));
    let (mut read, mut stored) = (0u64, 0u64);
    loop {
        let query = [("since", since.to_string()), ("limit", CHANGES_PAGE.to_string())];
        crate::log_info!("Capture device {} GET /api/store/changes {query:?}", device.id);
        let page: ChangesPage = serde_json::from_value(client.get_query("store/changes", &query).await?)?;
        let count = page.changes.len();
        for change in page.changes {
            let change = bruce_db::Change::from_json(change)?;
            if store.put_from(&change.item, &crate::store::device_part(&device.id))? == bruce_db::Put::Stored {
                stored += 1;
            }
        }
        read += count as u64;
        store.put(&bruce_db::Item::new(&hk, "-", crate::store::version_now(),
            serde_json::json!({ "store": page.store, "next": page.next })))?;
        if let Some(error) = page.error {
            anyhow::bail!("The capture device {} could not read all its changes, {error}", device.id);
        }
        // Only while the page moved on, so that a device which gives fewer than asked for is not asked forever.
        let moved = page.next != since;
        since = page.next;
        if count < CHANGES_PAGE || !moved {
            break;
        }
        progress(format!("Read {read} changes from the capture device {} so far, {stored} new", device.id));
    }
    crate::log_info!("Capture device {}: {read} changes, {stored} new, up to {since}", device.id);
    update_status(device, |status| {
        status.takes = info.features.keys().cloned().collect();
        status.last_read = Some(crate::clock::now_utc());
        status.last_error = None;
        status.read += read;
        status.saved += stored;
        status.device = info.features.values().next().cloned();
        for (module_id, feature) in &info.features {
            if let Some(poll) = &feature.last_poll {
                log_poll_change(status, module_id, status.polls.get(module_id), poll);
                status.polls.insert(module_id.clone(), poll.clone());
            }
        }
    });
    *latest = Some((std::time::Instant::now(), info.clone()));
    Ok(Some(info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_is_described_as_the_device_describes_it() {
        let chip: Chip = serde_json::from_value(serde_json::json!({
            "model": "ESP32-C6", "revision": "v0.2", "cores": 1, "flashBytes": 4194304, "psramBytes": 0,
        })).unwrap();
        assert_eq!(chip.describe(), "ESP32-C6 v0.2, 1 core, 4 MB flash, no PSRAM");
        let chip = Chip { model: "ESP32-S3".to_string(), revision: "v0.2".to_string(), cores: 2, flash_bytes: 16 << 20, psram_bytes: 8 << 20 };
        assert_eq!(chip.describe(), "ESP32-S3 v0.2, 2 cores, 16 MB flash, 8 MB PSRAM");
    }

    #[test]
    fn requests_are_signed_as_a_device_checks_them() {
        // From Python's hmac, as a check independent of the code.
        let key: String = (0u8..32).map(|byte| format!("{byte:02x}")).collect();
        let uri = "/api/capture/tesla/records?since=2026-10-30T06%3A00%3A00Z&limit=500";
        assert_eq!(sign(&key, "GET", uri, 1793340000).unwrap(),
            "Sparko-HMAC-SHA256 1793340000:e639c3cc64688ccbb3ae81570f79f7eab2944a7415544b40a701b9fe298de232");
    }

    #[test]
    fn the_next_read_is_a_minute_after_the_devices_poll_of_the_apps_next_slot() {
        let five_minutes = Duration::from_secs(5 * 60);
        let reader = CaptureReader::new("capture/tesla/records");
        let at = |text: &str| time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap();
        reader.schedules.lock().unwrap().insert("a".to_string(), "0 */5 * * * *".to_string());
        assert_eq!(reader.next_read(at("2026-10-29T17:02:10Z"), five_minutes), Some(at("2026-10-29T17:06:00Z")));
        // After a read, the next slot's.
        assert_eq!(reader.next_read(at("2026-10-29T17:06:00Z"), five_minutes), Some(at("2026-10-29T17:11:00Z")));
        // Every 5 minutes, however often the device polls.
        reader.schedules.lock().unwrap().insert("a".to_string(), "*/10 * * * * *".to_string());
        assert_eq!(reader.next_read(at("2026-10-29T17:02:10Z"), five_minutes), Some(at("2026-10-29T17:06:00Z")));
        // The soonest of two devices, one of which does not poll on the slots.
        reader.schedules.lock().unwrap().insert("a".to_string(), "30 29,59 * * * *".to_string());
        reader.schedules.lock().unwrap().insert("b".to_string(), "0 */15 * * * *".to_string());
        assert_eq!(reader.next_read(at("2026-10-29T17:16:00Z"), Duration::from_secs(15 * 60)), Some(at("2026-10-29T17:31:00Z")));
    }

    /// Against a real device, only reading: cargo test --lib capture_device -- --ignored, with CAPTURE_DEVICE set to its
    /// address.
    #[tokio::test]
    #[ignore]
    async fn finds_a_real_device() {
        let address = std::env::var("CAPTURE_DEVICE").expect("CAPTURE_DEVICE");
        let found = find(&address).await.unwrap();
        println!("{found:?}");
        let wrong = DeviceClient::new(&address, Some(&"0".repeat(64)), None).unwrap();
        let error = wrong.secure_info().await.unwrap_err();
        println!("with the wrong fingerprint: {error:#}");
        assert!(format!("{error:#}").contains("not the one it had"));
    }
}

/// For --deviceget: GETs path, which may have a query, e.g. capture/climate/records?limit=5, of the active profile's
/// capture device device, signed with the key the app keeps for it, which is not shown, and prints its response.
pub fn cli_get(args: &crate::Args, device_id: &str, path: &str) -> anyhow::Result<()> {
    let device = cli_device(args, device_id)?;
    let (path, query) = path.trim_start_matches('/').trim_start_matches("api/").split_once('?').unwrap_or((path, ""));
    let query: Vec<(String, String)> = reqwest::Url::parse(&format!("http://device/?{query}"))?.query_pairs().into_owned().collect();
    let query: Vec<(&str, String)> = query.iter().map(|(name, value)| (name.as_str(), value.clone())).collect();
    let client = DeviceClient::for_device(&device)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let json = runtime.block_on(client.get_query(path, &query))?;
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

/// For --devicepost: POSTs body, JSON, to path, e.g. config, of the active profile's capture device device, signed as
/// cli_get's are, and prints its response.
pub fn cli_post(args: &crate::Args, device_id: &str, path: &str, body: &str) -> anyhow::Result<()> {
    let device = cli_device(args, device_id)?;
    let body: Value = serde_json::from_str(body).map_err(|error| anyhow!("The body is not JSON: {error:#}"))?;
    let client = DeviceClient::for_device(&device)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let json = runtime.block_on(client.post(path.trim_start_matches('/').trim_start_matches("api/"), &body))?;
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

/// The active profile's capture device with the ID device_id, for --deviceget and --devicepost.
fn cli_device(args: &crate::Args, device_id: &str) -> anyhow::Result<DeviceProfile> {
    let profile = crate::profile::fetch_active_profile(&args.marco_sparko_args.profile)?;
    profile.active_profile.settings.capture_devices.iter()
        .find(|device| device.id == device_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("The profile {} has no capture device {device_id}", profile.active_profile.name))
}
