//! Requests to the myenergi API, which is not documented by myenergi, see https://github.com/twonk/MyEnergi-App-Api
//!
//! Every request is authenticated with HTTP digest authentication, with the hub's serial number as the user name and an
//! API key made in the myenergi app as the password. The director tells which server has the hub's data, in the
//! x_myenergi-asn header of its responses.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::anyhow;
use reqwest::StatusCode;
use time::Date;

use super::digest;

const DIRECTOR_HOST: &str = "director.myenergi.net";
const ASN_HEADER: &str = "x_myenergi-asn";

/// A myenergi device, e.g. a zappi, by the letter its requests use and its serial number.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// The kind of device, e.g. zappi, as the status names it.
    pub kind: String,
    /// The letter of its kind in requests, e.g. Z for a zappi.
    pub letter: char,
    pub serial: String,
}

impl Device {
    /// The devices in a cgi-jstatus-* response, an array of objects such as {"zappi": [{"sno": 12345678, ...}]}.
    pub fn from_status(status: &serde_json::Value) -> Vec<Device> {
        let mut devices = Vec::new();
        for item in status.as_array().into_iter().flatten() {
            for (kind, list) in item.as_object().into_iter().flatten() {
                let letter = match kind.as_str() {
                    "zappi" => 'Z',
                    "eddi" => 'E',
                    "harvi" => 'H',
                    "libbi" => 'L',
                    _ => continue,
                };
                for device in list.as_array().into_iter().flatten() {
                    // A number, but a string would do as well.
                    let serial = match device.get("sno") {
                        Some(serde_json::Value::Number(number)) => number.to_string(),
                        Some(serde_json::Value::String(text)) => text.clone(),
                        _ => continue,
                    };
                    devices.push(Device {
                        kind: kind.clone(),
                        letter,
                        serial,
                    });
                }
            }
        }
        devices
    }
}

pub struct MyenergiClient {
    http: reqwest::Client,
    hub_serial: String,
    api_key: String,
    /// The server with the hub's data, the director until it has said which.
    host: Mutex<String>,
    /// The digest requests made so far, to number them, see digest::authorization.
    count: AtomicU32,
    verbose: bool,
}

impl MyenergiClient {
    /// host is the server the director said has the hub's data, if known, see host.
    pub fn new(
        hub_serial: String,
        api_key: String,
        host: Option<String>,
        verbose: bool,
    ) -> anyhow::Result<MyenergiClient> {
        Ok(MyenergiClient {
            http: crate::http::client_builder().build()?,
            hub_serial,
            api_key,
            host: Mutex::new(host.unwrap_or_else(|| DIRECTOR_HOST.to_string())),
            count: AtomicU32::new(0),
            verbose,
        })
    }

    /// The server with the hub's data, which is the director until it has answered a request.
    pub fn host(&self) -> String {
        self.host.lock().unwrap().clone()
    }

    /// GETs path, e.g. /cgi-jstatus-*, answering the server's digest challenge. A request without authentication gets
    /// the challenge, as the nonce may only be good for one request.
    async fn get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("https://{}{path}", self.host());
        crate::log_info!("myenergi GET {path}");
        let response = self.http.get(&url).send().await?;
        let response = if response.status() == StatusCode::UNAUTHORIZED {
            let challenge = response
                .headers()
                .get(reqwest::header::WWW_AUTHENTICATE)
                .and_then(|header| header.to_str().ok())
                .and_then(digest::parse_challenge)
                .ok_or_else(|| anyhow!("myenergi asked for authentication, but not digest authentication"))?;
            let nc = self.count.fetch_add(1, Ordering::SeqCst) + 1;
            let cnonce = format!("{:016x}", rand::random::<u64>());
            let authorization =
                digest::authorization(&challenge, &self.hub_serial, &self.api_key, "GET", path, nc, &cnonce);
            self.http
                .get(&url)
                .header(reqwest::header::AUTHORIZATION, authorization)
                .send()
                .await?
        } else {
            response
        };

        let status = response.status();
        if let Some(asn) = response
            .headers()
            .get(ASN_HEADER)
            .and_then(|header| header.to_str().ok())
        {
            let asn = asn.trim();
            if !asn.is_empty() && asn != *self.host.lock().unwrap() {
                crate::log_info!("myenergi data is on {asn}");
                *self.host.lock().unwrap() = asn.to_string();
            }
        }
        let text = response.text().await?;
        if status == StatusCode::UNAUTHORIZED {
            return Err(anyhow!("myenergi did not accept the hub serial number and API key"));
        }
        if !status.is_success() {
            return Err(anyhow!("myenergi responded {status}: {}", crate::http::summary(&text)));
        }
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
            anyhow!(
                "myenergi responded with something which is not JSON, {error}: {}",
                crate::http::summary(&text)
            )
        })?;
        Ok(json)
    }

    /// The status of every device, which also finds the server with the hub's data, see host.
    pub async fn status(&self) -> anyhow::Result<serde_json::Value> {
        let path = "/cgi-jstatus-*";
        let json = self.get(path).await?;
        crate::http::print_response(self.verbose, &format!("myenergi {path}"), &json);
        Ok(json)
    }

    /// The device's records of each minute of the UTC day date, an object holding an array of them, e.g.
    /// {"U12345678": [{"yr": 2026, "mon": 10, "dom": 27, "hr": 0, "min": 1, "imp": 6120, ...}]}. Energy is in joules
    /// in the minute, imp imported, exp exported, gep generated, h1d, h2d and h3d used by the device on each phase.
    pub async fn day(&self, device: &Device, date: Date) -> anyhow::Result<serde_json::Value> {
        self.records(&format!(
            "/cgi-jday-{}{}-{}-{}-{}",
            device.letter,
            device.serial,
            date.year(),
            u8::from(date.month()),
            date.day()
        ))
        .await
    }

    /// The device's records of count minutes of the UTC day date from hour:minute, as day does for the whole day.
    pub async fn part_of_day(
        &self,
        device: &Device,
        date: Date,
        hour: u8,
        minute: u8,
        count: u16,
    ) -> anyhow::Result<serde_json::Value> {
        self.records(&format!(
            "/cgi-jday-{}{}-{}-{}-{}-{hour}-{minute}-{count}",
            device.letter,
            device.serial,
            date.year(),
            u8::from(date.month()),
            date.day()
        ))
        .await
    }

    async fn records(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let json = self.get(path).await?;
        // Only how many records, as a day's are too many to read in the verbose output.
        if self.verbose {
            let records = json
                .as_object()
                .and_then(|object| object.values().next())
                .and_then(|records| records.as_array())
                .map_or(0, Vec::len);
            println!("myenergi {path}: {records} records");
        }
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_found_in_the_status() {
        let status = serde_json::json!([
            {"eddi": []},
            {"zappi": [{"sno": 12345678, "dat": "28-10-2026"}]},
            {"harvi": [{"sno": "87654321"}]},
            {"asn": "s18.myenergi.net", "fwv": "3401S5.041"}
        ]);
        assert_eq!(
            Device::from_status(&status),
            vec![
                Device {
                    kind: "zappi".to_string(),
                    letter: 'Z',
                    serial: "12345678".to_string()
                },
                Device {
                    kind: "harvi".to_string(),
                    letter: 'H',
                    serial: "87654321".to_string()
                },
            ]
        );
    }
}
