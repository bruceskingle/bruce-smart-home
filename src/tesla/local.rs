//! Requests to the local API of a Tesla Powerwall's gateway, on the home network, which is not documented by Tesla,
//! see https://github.com/jasonacox/pypowerwall
//!
//! Other programs, e.g. Home Assistant, use the gateway too, so this must never get in their way:
//! - Only the GET requests in ENDPOINTS are made, which change nothing.
//! - It logs in as the customer, which cannot change installer settings, and reuses the token it is given.
//! - A rejected password is never tried again, as repeated failures lock out the customer login, for every program.

use std::net::{Ipv4Addr, UdpSocket};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use reqwest::StatusCode;
use serde::Serialize;

/// The requests which may be made, after /api/, all of which only read.
pub const ENDPOINTS: &[&str] = &[
    "status",
    "site_info",
    "meters/aggregates",
    "meters/site",
    "meters/solar",
    "system_status",
    "system_status/soe",
    "system_status/grid_status",
    "powerwalls",
    "operation",
];

/// The gateway did not accept the email and password, which must not be tried again until they are changed.
#[derive(Debug)]
pub struct PasswordRejected;

impl std::fmt::Display for PasswordRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the gateway did not accept the email and password, check them before trying again, as repeated failures lock out the customer login for a while, for other programs such as Home Assistant too"
        )
    }
}

impl std::error::Error for PasswordRejected {}

#[derive(Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    email: &'a str,
    password: &'a str,
    /// Always false, as true stops the system on some versions of the gateway's software.
    force_sm_off: bool,
}

/// A client for a gateway, which only accepts its own certificate, see client_builder.
fn client_builder() -> reqwest::ClientBuilder {
    // The gateway's certificate is one it made itself, so it cannot be checked. Only the gateway's address is ever
    // requested with this client.
    // TODO: pin the certificate the gateway gave the first time instead.
    crate::http::client_builder().danger_accept_invalid_certs(true)
}

pub struct LocalClient {
    http: reqwest::Client,
    host: Mutex<String>,
    email: String,
    password: String,
    token: Mutex<Option<String>>,
    /// Whether this has logged in, which it does at most once, so that a rejected password is not tried again.
    logged_in: AtomicBool,
    verbose: bool,
}

impl LocalClient {
    /// token is one the gateway gave before, if any, which is used until it is rejected.
    pub fn new(
        host: String,
        email: String,
        password: String,
        token: Option<String>,
        verbose: bool,
    ) -> anyhow::Result<LocalClient> {
        Ok(LocalClient {
            http: client_builder().build()?,
            host: Mutex::new(host),
            email,
            password,
            token: Mutex::new(token),
            logged_in: AtomicBool::new(false),
            verbose,
        })
    }

    pub fn host(&self) -> String {
        self.host.lock().unwrap().clone()
    }

    /// Uses the gateway at host from now on, e.g. one found by find_gateway after its address changed.
    pub fn set_host(&self, host: String) {
        *self.host.lock().unwrap() = host;
    }

    /// The token the gateway gave, to save it for next time.
    pub fn token(&self) -> Option<String> {
        self.token.lock().unwrap().clone()
    }

    fn url(&self, endpoint: &str) -> String {
        format!("https://{}/api/{endpoint}", self.host())
    }

    /// Logs in as the customer, once at most, see PasswordRejected.
    pub async fn login(&self) -> anyhow::Result<()> {
        if self.logged_in.swap(true, Ordering::SeqCst) {
            return Err(anyhow!("the gateway rejected the token it gave when this logged in"));
        }
        crate::log_info!("Tesla gateway POST login/Basic");
        let request = LoginRequest {
            username: "customer",
            email: &self.email,
            password: &self.password,
            force_sm_off: false,
        };
        let response = self.http.post(self.url("login/Basic")).json(&request).send().await?;
        let status = response.status();
        let text = response.text().await?;
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Err(PasswordRejected.into()),
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(anyhow!(
                    "the gateway has locked out the customer login after too many attempts, wait before trying again"
                ));
            }
            status if !status.is_success() => {
                return Err(anyhow!(
                    "the gateway responded {status}: {}",
                    crate::http::summary(&text)
                ));
            }
            _ => {}
        }
        let json: serde_json::Value = serde_json::from_str(&text)?;
        crate::http::print_response(self.verbose, "Tesla gateway login/Basic", &json);
        let token = json
            .get("token")
            .and_then(|token| token.as_str())
            .ok_or_else(|| anyhow!("the gateway's login response has no token"))?;
        *self.token.lock().unwrap() = Some(token.to_string());
        Ok(())
    }

    /// GETs one of the ENDPOINTS, logging in first if there is no token, or again if the token is rejected.
    pub async fn get(&self, endpoint: &str) -> anyhow::Result<serde_json::Value> {
        if !ENDPOINTS.contains(&endpoint) {
            return Err(anyhow!(
                "{endpoint} is not one of the gateway's endpoints which only read"
            ));
        }
        if self.token().is_none() {
            self.login().await?;
        }
        let mut response = self.send_get(endpoint).await?;
        if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
            && !self.logged_in.load(Ordering::SeqCst)
        {
            crate::log_info!("The Tesla gateway did not accept the saved token, logging in again");
            self.login().await?;
            response = self.send_get(endpoint).await?;
        }
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "the gateway responded {status} to {endpoint}: {}",
                crate::http::summary(&text)
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&text)?;
        crate::http::print_response(self.verbose, &format!("Tesla gateway {endpoint}"), &json);
        Ok(json)
    }

    async fn send_get(&self, endpoint: &str) -> anyhow::Result<reqwest::Response> {
        crate::log_info!("Tesla gateway GET {endpoint}");
        let mut request = self.http.get(self.url(endpoint));
        if let Some(token) = self.token() {
            request = request.bearer_auth(token);
        }
        Ok(request.send().await?)
    }
}

/// The address of this computer on the home network, from the route it would use to reach the internet, which sends
/// nothing.
fn local_ipv4() -> anyhow::Result<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    match socket.local_addr()?.ip() {
        std::net::IpAddr::V4(address) => Ok(address),
        std::net::IpAddr::V6(address) => Err(anyhow!("this computer's address, {address}, is not IPv4")),
    }
}

/// Whether there is a gateway at host, by its status, which needs no login, and is the gateway with the serial number
/// din, if given.
async fn is_gateway(http: &reqwest::Client, host: &str, din: Option<&str>) -> Option<serde_json::Value> {
    let status: serde_json::Value = http
        .get(format!("https://{host}/api/status"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let is_gateway = status.get("device_type").and_then(|value| value.as_str()) == Some("teg");
    let is_din = din.is_none_or(|din| status.get("din").and_then(|value| value.as_str()) == Some(din));
    (is_gateway && is_din).then_some(status)
}

/// Finds a gateway on the home network, the one with the serial number din if given, by asking every address in a /24
/// subnet for its status: first the subnet of near, e.g. where the gateway was last, as it may be on another network
/// than this computer, e.g. one for IoT devices, then this computer's. Returns its address and status.
pub async fn find_gateway(
    din: Option<&str>,
    near: Option<&str>,
) -> anyhow::Result<Option<(String, serde_json::Value)>> {
    let mut subnets = Vec::new();
    if let Some(address) = near.and_then(|near| near.parse::<Ipv4Addr>().ok()) {
        subnets.push(address);
    }
    let local = local_ipv4()?;
    if !subnets.iter().any(|subnet| subnet.octets()[..3] == local.octets()[..3]) {
        subnets.push(local);
    }
    let http = client_builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()?;
    for subnet in subnets {
        let [a, b, c, _] = subnet.octets();
        crate::log_info!("Searching {a}.{b}.{c}.0/24 for the Tesla gateway");
        let mut probes = futures::stream::iter((1..=254u8).map(|d| format!("{a}.{b}.{c}.{d}")))
            .map(|host| {
                let http = &http;
                async move { is_gateway(http, &host, din).await.map(|status| (host, status)) }
            })
            .buffer_unordered(64);
        while let Some(found) = probes.next().await {
            if let Some((host, status)) = found {
                crate::log_info!("Found the Tesla gateway at {host}");
                return Ok(Some((host, status)));
            }
        }
    }
    Ok(None)
}
