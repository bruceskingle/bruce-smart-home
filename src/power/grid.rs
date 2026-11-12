//! Readings of the grid every few seconds, e.g. of the Octopus Home Mini, which a module gives, see GridSample, and
//! their totals over periods, see aggregate.

use bruce_graphql::types::DateTime;
use time_tz::Tz;

use crate::period::{Period, calendar_bounds};

/// How long a reading with none before it is taken to be, in seconds, as grid meters are read about every 10 seconds.
pub const READING_SECONDS: i64 = 10;

/// A reading of a grid meter, as a module has it.
pub trait GridSample {
    /// When it was read, a unix timestamp.
    fn read_at(&self) -> i64;
    /// The power at the time, in W, negative when exporting.
    fn demand(&self) -> Option<f64>;
    /// The energy imported since the reading before, in Wh.
    fn consumption_delta(&self) -> Option<f64>;
    /// The meter's import and export registers, in Wh.
    fn consumption(&self) -> Option<f64>;
    fn export(&self) -> Option<f64>;
}

/// A reading of a grid meter, for the Electricity page, see power::SiteData.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridReading {
    pub read_at: i64,
    pub demand: Option<f64>,
    pub consumption_delta: Option<f64>,
    pub consumption: Option<f64>,
    pub export: Option<f64>,
}

impl GridReading {
    /// The same reading as a GridReading.
    pub fn from_sample(sample: &impl GridSample) -> GridReading {
        GridReading {
            read_at: sample.read_at(),
            demand: sample.demand(),
            consumption_delta: sample.consumption_delta(),
            consumption: sample.consumption(),
            export: sample.export(),
        }
    }
}

impl GridSample for GridReading {
    fn read_at(&self) -> i64 {
        self.read_at
    }

    fn demand(&self) -> Option<f64> {
        self.demand
    }

    fn consumption_delta(&self) -> Option<f64> {
        self.consumption_delta
    }

    fn consumption(&self) -> Option<f64> {
        self.consumption
    }

    fn export(&self) -> Option<f64> {
        self.export
    }
}

/// The readings in a period, totalled.
#[derive(Clone, PartialEq, Debug)]
pub struct GridPoint {
    /// Unix timestamps.
    pub start: i64,
    pub end: i64,
    /// The energy imported, in Wh, see energy_since.
    pub energy: f64,
    /// The energy exported, in Wh, see export_since.
    pub export: f64,
    /// The average power, in W, negative when more is exported than imported, None if there were no readings of it.
    pub demand: Option<f64>,
    /// The number of readings.
    pub readings: usize,
}

/// The energy imported up to reading since previous, the reading before it, in Wh. This is the change in the meter's
/// import register, consumption, which is exact however far apart the readings are, so readings which are missing do
/// not lose any energy, it is in the reading after them. A reading's consumption_delta is the energy since the reading
/// before, and usually covers a reading which is missing too, but not always, so it is only used when there are no
/// register readings, e.g. for the first reading, or the register goes down, e.g. if the meter is replaced.
pub fn energy_since(previous: Option<&impl GridSample>, reading: &impl GridSample) -> f64 {
    match (previous.and_then(|previous| previous.consumption()), reading.consumption()) {
        (Some(before), Some(after)) if after >= before => after - before,
        _ => reading.consumption_delta().unwrap_or_default(),
    }
}

/// The energy exported, in Wh, since the reading before, from the export register as for energy_since, or 0 if there
/// is not one before or it goes down, as there is no export delta.
fn export_since(previous: Option<&impl GridSample>, reading: &impl GridSample) -> f64 {
    match (previous.and_then(|previous| previous.export()), reading.export()) {
        (Some(before), Some(after)) if after >= before => after - before,
        _ => 0.0,
    }
}

/// A point for each reading from start to end, from the reading before it to its time, which is the time its energy was
/// used in, see energy_since, so that they meet although some 10 second slots have no reading, e.g. the Octopus Home Mini's. A
/// reading with none before it is 10 seconds long. Readings before start are not included, but the last of them gives
/// the energy and start of the first reading from start.
fn reading_points<R: GridSample>(readings: &[R], start: i64, end: i64) -> Vec<GridPoint> {
    let mut points: Vec<GridPoint> = Vec::new();
    let mut previous: Option<&R> = None;
    for reading in readings.iter().take_while(|reading| reading.read_at() < end) {
        let since = previous.replace(reading);
        if reading.read_at() < start {
            continue;
        }
        points.push(GridPoint {
            start: since.map(|since| since.read_at()).unwrap_or(reading.read_at() - READING_SECONDS),
            end: reading.read_at(),
            energy: energy_since(since, reading),
            export: export_since(since, reading),
            demand: reading.demand(),
            readings: 1,
        });
    }
    points
}

/// The periods from start to end, as unix timestamps, including ones with no readings, with the readings, which are in
/// time order, totalled in each. Readings before start are not counted, but the last of them gives the energy of the
/// first reading from start. For 10 seconds there is a period for each reading, see reading_points.
pub fn aggregate<R: GridSample>(readings: &[R], start: i64, end: i64, period: Period, tz: &'static Tz) -> Vec<GridPoint> {
    if period == Period::TenSeconds {
        return reading_points(readings, start, end);
    }
    let mut points = Vec::new();
    let mut period_start = match period.seconds() {
        Some(seconds) => start - start.rem_euclid(seconds),
        None => match DateTime::from_unix_timestamp(start) {
            Ok(start) => calendar_bounds(start.to_timezone(tz).to_date(), period, tz).0,
            Err(_) => return points,
        },
    };
    let mut readings = readings.iter().peekable();
    // The reading before the next, for the energy since it.
    let mut previous: Option<&R> = None;
    while period_start < end {
        let period_end = match period.seconds() {
            Some(seconds) => period_start + seconds,
            None => match DateTime::from_unix_timestamp(period_start) {
                Ok(time) => calendar_bounds(time.to_timezone(tz).to_date(), period, tz).1,
                Err(_) => break,
            },
        };
        let mut point = GridPoint { start: period_start, end: period_end, energy: 0.0, export: 0.0, demand: None, readings: 0 };
        let mut demand_total = 0.0;
        let mut demand_readings = 0;
        while let Some(reading) = readings.next_if(|reading| reading.read_at() < period_end) {
            let since = previous.replace(reading);
            if reading.read_at() < period_start || reading.read_at() < start {
                continue;
            }
            point.readings += 1;
            point.energy += energy_since(since, reading);
            point.export += export_since(since, reading);
            if let Some(demand) = reading.demand() {
                demand_total += demand;
                demand_readings += 1;
            }
        }
        if demand_readings > 0 {
            point.demand = Some(demand_total / demand_readings as f64);
        }
        points.push(point);
        period_start = period_end;
    }
    points
}
