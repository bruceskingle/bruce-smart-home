//! Logs the board's own temperature and humidity sensor, e.g. the Waveshare ESP32-S3-RLCD-4.2's SHTC3, read every 10
//! seconds, and its battery's voltage, see bruce_esp_idf::battery, as a third series, which the app shows on the
//! device's card rather than with the temperature and humidity, as it is about the device. Unlike the other features' records, which are copies of what a source said, these are the readings
//! themselves, which no one else has, so each device's are its own.
//!
//! As they change slowly, each of temperature and humidity is kept as periods: a start, an end and a value, which lasts
//! while the readings stay within STEP of the value, see Series, rather than a record every 10 seconds. A period is
//! written when it closes, and while it is open once a minute, with its end moved on, so that the app, which saves
//! each period by its start, replaces what it had of it, and a restart loses at most a minute of it. The records are
//! the lines {"series": "temperature", "start", "end", "value", "unit", "open", "capturedBy"}, put in the store as the
//! app would put them, see db.rs.

use std::sync::{Arc, Mutex};

use bruce_embedded_std::config::ConfigSpec;
use bruce_embedded_std::config::ConfigSpecValue;
use bruce_embedded_std::config::FeatureConfig;
use bruce_embedded_std::config::TypedValue;
use bruce_embedded_std::feature::FeatureDescriptor;
use bruce_embedded_std::platform::PlatformInitializer;
use bruce_embedded_std::task::scheduler::ScheduledTask;
use bruce_esp_idf::{Esp32Platform, Esp32PlatformInitializer, Feature};
use chrono::{DateTime, TimeDelta, Utc};
use croner::Cron;
use log::warn;
use serde_json::json;

use crate::db::Db;
use crate::device::{Device, captured_by};
use crate::records::{LastPoll, format_time};
use crate::screen::Panel;

/// The feature's name, which is also its NVS namespace, so at most 15 characters.
const FEATURE_NAME: &str = "ClimateCapture";

/// Every 10 seconds.
const DEFAULT_SCHEDULE: &str = "*/10 * * * * *";

/// How often an open period is written, with its end moved on.
const WRITE_OPEN_EVERY: TimeDelta = TimeDelta::minutes(1);

/// Where its line goes on the screen, after the polled features'.
const PANEL_ORDER: u8 = 4;

#[derive(FeatureConfig)]
#[config(label = "Temperature and Humidity Log")]
pub struct ClimateConfig {
    #[config(label = "Climate Sample Schedule (cron, seconds first; every 10 seconds if empty)")]
    climate_sched: Option<Cron>,
}

pub struct ClimateCapture {
    panel: Arc<Panel>,
    db: Arc<Db>,
}

impl ClimateCapture {
    pub fn new(panel: Arc<Panel>, db: Arc<Db>) -> Self {
        Self { panel, db }
    }
}

impl Feature for ClimateCapture {
    type Config = ClimateConfig;

    fn init(&self, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<FeatureDescriptor> {
        self.panel.start(initializer)?;
        Ok(FeatureDescriptor {
            name: FEATURE_NAME.to_string(),
            config: ClimateConfig::to_config_spec()?,
        })
    }

    fn start(
        &mut self,
        sparko: &mut Esp32Platform,
        initializer: &mut Esp32PlatformInitializer,
        config: ClimateConfig,
    ) -> anyhow::Result<()> {
        let schedule_spec = config
            .climate_sched
            .map(|schedule| schedule.pattern.to_string())
            .unwrap_or_else(|| DEFAULT_SCHEDULE.to_string());
        let store = Arc::new(Mutex::new(Store { last_poll: None }));
        // The app reads every 5 minutes, see bruce-smart-home's src/climate.rs, not on every sample.
        let device = Arc::new(Device::new("0 */5 * * * *"));

        let sample = SampleTask {
            sensor: sparko.shtc3(),
            series: [
                Series::new("temperature", "°C", 0.1, 1),
                Series::new("humidity", "%", 1.0, 0),
            ],
            // A step of 0.02 V, twice what is shown, so that its jitter of a hundredth does not make a period of each
            // reading.
            battery: sparko
                .battery()
                .map(|battery| (battery, Series::new("battery", "V", 0.02, 2))),
            store: store.clone(),
            db: self.db.clone(),
            panel: self.panel.clone(),
            captured_by: captured_by(sparko),
        };
        initializer.add_task(Box::new(sample), &schedule_spec)?;

        // For the app, at /api/store/info, see db.rs.
        self.db.add_status("climate", move |db| {
            device.to_json(db, store.lock().unwrap().last_poll.as_ref())
        });
        Ok(())
    }
}

/// Puts a period's record in the store, by the device's ID.
fn put(db: &Db, record: &serde_json::Value) {
    let device = db.device();
    db.put(|store, _| crate::device_kinds::climate_period(store, &device, record));
}

struct Store {
    /// How the latest reading went, for the app, see Device::to_json.
    last_poll: Option<LastPoll>,
}

/// A period: from start to end the readings were within the series' step of value.
struct Period {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    /// In f64, rounded, so that it is in the JSON as it is shown, e.g. 19.6, not as the f32 nearest it, 19.600000381.
    value: f64,
    /// When it was last written, see WRITE_OPEN_EVERY.
    written: Option<DateTime<Utc>>,
}

/// One of the sensor's measurements, e.g. temperature, as periods, see Period.
struct Series {
    name: &'static str,
    unit: &'static str,
    /// How far a reading must be from the open period's value to start a new one, e.g. 0.1 °C, so that the sensor's
    /// jitter, a few hundredths, around a value does not start a period every reading.
    step: f32,
    /// The decimal places a value is rounded to, e.g. 1, 23.4 °C.
    decimals: i32,
    open: Option<Period>,
}

impl Series {
    fn new(name: &'static str, unit: &'static str, step: f32, decimals: i32) -> Series {
        Series {
            name,
            unit,
            step,
            decimals,
            open: None,
        }
    }

    fn round(&self, reading: f32) -> f64 {
        let scale = 10f64.powi(self.decimals);
        (reading as f64 * scale).round() / scale
    }

    /// Adds a reading at time, returning the records to write: the period it closes, if it closes one, and the open
    /// period, if it is new or has not been written for WRITE_OPEN_EVERY.
    fn add(&mut self, time: DateTime<Utc>, reading: f32) -> Vec<(Period, bool)> {
        let mut records = Vec::new();
        match &mut self.open {
            // The tolerance, as the step is a whole number of the value's decimals, which floats do not quite hold.
            Some(open) if (reading as f64 - open.value).abs() < self.step as f64 - 1e-3 => open.end = time,
            _ => {
                if let Some(mut closed) = self.open.take() {
                    // Up to the reading which closed it, so that the periods are one after another.
                    closed.end = time;
                    records.push((closed, false));
                }
                self.open = Some(Period {
                    start: time,
                    end: time,
                    value: self.round(reading),
                    written: None,
                });
            }
        }
        if let Some(open) = &mut self.open
            && open.written.is_none_or(|written| time - written >= WRITE_OPEN_EVERY)
        {
            open.written = Some(time);
            records.push((Period { written: None, ..*open }, true));
        }
        records
    }

    fn to_json(&self, period: &Period, open: bool, captured_by: &str) -> serde_json::Value {
        json!({
            "series": self.name,
            "start": format_time(period.start),
            "end": format_time(period.end),
            "value": period.value,
            "unit": self.unit,
            "open": open,
            "capturedBy": captured_by,
        })
    }
}

struct SampleTask {
    sensor: bruce_esp_idf::shtc3::Shtc3,
    /// Temperature and humidity.
    series: [Series; 2],
    /// The board's battery, if it can be read, and its voltage.
    battery: Option<(Arc<bruce_esp_idf::battery::Battery>, Series)>,
    store: Arc<Mutex<Store>>,
    db: Arc<Db>,
    panel: Arc<Panel>,
    captured_by: String,
}

impl ScheduledTask<Esp32Platform> for SampleTask {
    fn run(&mut self, _sparko: &mut Esp32Platform) -> anyhow::Result<()> {
        let time = Utc::now();
        // The battery whatever the sensor does, as it says how the board is powered.
        if let Some((battery, series)) = &mut self.battery {
            match battery.read() {
                Ok(reading) => {
                    for (period, open) in series.add(time, reading.volts) {
                        put(&self.db, &series.to_json(&period, open, &self.captured_by));
                    }
                }
                Err(error) => warn!("Unable to read the battery, {error:#}"),
            }
        }
        let reading = match self.sensor.read() {
            Ok(reading) => reading,
            Err(error) => {
                warn!("Unable to read the temperature and humidity, {error:#}");
                let mut store = self.store.lock().unwrap();
                store.last_poll = Some(LastPoll::failed(store.last_poll.as_ref(), time, format!("{error:#}")));
                self.panel
                    .set(PANEL_ORDER, vec![(format!("Temp/humidity: {error:#}"), true)]);
                return Ok(());
            }
        };
        let mut store = self.store.lock().unwrap();
        for (series, value) in self.series.iter_mut().zip([reading.celsius, reading.humidity]) {
            for (period, open) in series.add(time, value) {
                put(&self.db, &series.to_json(&period, open, &self.captured_by));
            }
        }
        store.last_poll = Some(LastPoll::succeeded(time));
        self.panel.set(
            PANEL_ORDER,
            vec![(
                format!("{:.1} C  {:.0}% humidity", reading.celsius, reading.humidity),
                false,
            )],
        );
        Ok(())
    }

    fn name(&self) -> &str {
        "Climate Capture"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_period_lasts_while_the_readings_stay_within_a_step() {
        let at = |seconds: i64| DateTime::from_timestamp(1_800_000_000 + seconds, 0).unwrap();
        let mut series = Series::new("temperature", "°C", 0.1, 1);
        // The first reading opens a period, which is written at once.
        let records = series.add(at(0), 23.44);
        assert_eq!(records.len(), 1);
        assert!(records[0].1 && records[0].0.value == 23.4);
        // Jitter within the step only moves the end on.
        assert!(series.add(at(10), 23.47).is_empty());
        assert!(series.add(at(20), 23.36).is_empty());
        // A step away closes it at that reading, and opens the next.
        let records = series.add(at(30), 23.52);
        assert_eq!(records.len(), 2);
        assert!(!records[0].1 && records[0].0.end == at(30) && records[0].0.value == 23.4);
        assert!(records[1].1 && records[1].0.start == at(30) && records[1].0.value == 23.5);
        // An open period is written again a minute after it last was.
        assert!(series.add(at(80), 23.5).is_empty());
        let records = series.add(at(90), 23.5);
        assert_eq!(records.len(), 1);
        assert!(records[0].1 && records[0].0.end == at(90));
    }
}
