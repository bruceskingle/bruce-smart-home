//! Logs the board's own temperature and humidity sensor, e.g. the Waveshare ESP32-S3-RLCD-4.2's SHTC3, read every 10
//! seconds. Unlike the other features' records, which are copies of what a source said, these are the readings
//! themselves, which no one else has, so each device's are its own.
//!
//! As they change slowly, each of temperature and humidity is kept as periods: a start, an end and a value, which lasts
//! while the readings stay within STEP of the value, see Series, rather than a record every 10 seconds. A period is
//! written when it closes, and while it is open once a minute, with its end moved on, so that the app, which saves
//! each period by its start, replaces what it had of it, and a restart loses at most a minute of it. The records are
//! the lines {"series": "temperature", "start", "end", "value", "unit", "open", "capturedBy"}, on the SD card, see
//! DayFiles, in the files of the days of their ends, and the latest in RAM too.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeDelta, Utc};
use croner::Cron;
use log::warn;
use serde_json::json;
use sparko_embedded_std::config::ConfigSpec;
use sparko_embedded_std::config::ConfigSpecValue;
use sparko_embedded_std::config::FeatureConfig;
use sparko_embedded_std::config::TypedValue;
use sparko_embedded_std::feature::FeatureDescriptor;
use sparko_embedded_std::platform::PlatformInitializer;
use sparko_embedded_std::task::scheduler::ScheduledTask;
use sparko_esp_idf::{Esp32Platform, Esp32PlatformInitializer, Feature};

use crate::day_files::{self, DayFiles};
use crate::device::{Device, captured_by};
use crate::records::{LastPoll, format_time};
use crate::screen::Panel;

/// The feature's name, which is also its NVS namespace, so at most 15 characters.
const FEATURE_NAME: &str = "ClimateCapture";

/// Every 10 seconds.
const DEFAULT_SCHEDULE: &str = "*/10 * * * * *";

/// How often an open period is written, with its end moved on.
const WRITE_OPEN_EVERY: TimeDelta = TimeDelta::minutes(1);

/// About 2 days of records, two a minute and those which close, for when there is no SD card, which only a board with
/// PSRAM, e.g. the RLCD, has room for.
const RAM_CAPACITY: usize = 6000;

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
}

impl ClimateCapture {
    pub fn new(panel: Arc<Panel>) -> Self {
        Self { panel }
    }
}

impl Feature for ClimateCapture {
    type Config = ClimateConfig;

    fn init(&self, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<FeatureDescriptor> {
        self.panel.start(initializer)?;
        Ok(FeatureDescriptor { name: FEATURE_NAME.to_string(), config: ClimateConfig::to_config_spec()? })
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
        let dir = sparko.sd_card().map(|mount_point| std::path::Path::new(mount_point).join("capture").join("climate"));
        let store = Arc::new(Mutex::new(Store {
            files: DayFiles::new(dir),
            ram: VecDeque::with_capacity(RAM_CAPACITY),
            last_poll: None,
        }));
        // The app reads every 5 minutes, see marco-sparko's src/climate.rs, not on every sample.
        let device = Arc::new(Device::new("0 */5 * * * *"));

        let sample = SampleTask {
            sensor: sparko.shtc3(),
            series: [Series::new("temperature", "°C", 0.1, 1), Series::new("humidity", "%", 1.0, 0)],
            store: store.clone(),
            panel: self.panel.clone(),
            captured_by: captured_by(sparko),
        };
        initializer.add_task(Box::new(sample), &schedule_spec)?;

        sparko.api_get("/api/capture/climate/records", move |request| {
            let (files, device_json) = {
                let store = store.lock().unwrap();
                (store.files.usable(), device.to_json(&store.files, store.ram.len(), RAM_CAPACITY, store.last_poll.as_ref()))
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
    /// The latest records, oldest first, by their ends, as their JSON.
    ram: VecDeque<(DateTime<Utc>, Box<str>)>,
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
        Series { name, unit, step, decimals, open: None }
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
                self.open = Some(Period { start: time, end: time, value: self.round(reading), written: None });
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

    fn to_json(&self, period: &Period, open: bool, captured_by: &str) -> String {
        json!({
            "series": self.name,
            "start": format_time(period.start),
            "end": format_time(period.end),
            "value": period.value,
            "unit": self.unit,
            "open": open,
            "capturedBy": captured_by,
        })
        .to_string()
    }
}

struct SampleTask {
    sensor: sparko_esp_idf::shtc3::Shtc3,
    /// Temperature and humidity.
    series: [Series; 2],
    store: Arc<Mutex<Store>>,
    panel: Arc<Panel>,
    captured_by: String,
}

impl ScheduledTask<Esp32Platform> for SampleTask {
    fn run(&mut self, _sparko: &mut Esp32Platform) -> anyhow::Result<()> {
        let time = Utc::now();
        let reading = match self.sensor.read() {
            Ok(reading) => reading,
            Err(error) => {
                warn!("Unable to read the temperature and humidity, {error:#}");
                let mut store = self.store.lock().unwrap();
                store.last_poll = Some(LastPoll::failed(store.last_poll.as_ref(), time, format!("{error:#}")));
                self.panel.set(PANEL_ORDER, vec![(format!("Temp/humidity: {error:#}"), true)]);
                return Ok(());
            }
        };
        let mut store = self.store.lock().unwrap();
        for (series, value) in self.series.iter_mut().zip([reading.celsius, reading.humidity]) {
            for (period, open) in series.add(time, value) {
                let record = series.to_json(&period, open, &self.captured_by);
                store.files.append(period.end, &record);
                let Store { files, ram, .. } = &mut *store;
                files.keep(ram, RAM_CAPACITY, period.end, record);
            }
        }
        store.last_poll = Some(LastPoll::succeeded(time));
        self.panel.set(PANEL_ORDER, vec![(format!("{:.1} C  {:.0}% humidity", reading.celsius, reading.humidity), false)]);
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
