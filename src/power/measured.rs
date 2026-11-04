//! What a site's own devices measured of where its energy went, e.g. a Tesla gateway's energy registers, which a
//! supplier's module, e.g. octopus/electricity.rs, uses to split its readings of the grid's import by what it was used
//! for, see Measurements, without knowing where they are from.
//!
//! The split needs the grid's import measured outside the gateway, e.g. by the Octopus Home Mini, as the car's charger
//! may be outside the gateway's site meter: then the car's import is the grid's less the gateway's site meter's, the
//! batteries' what they took in less what the solar panels made, and the rest the house's.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

const HALF_HOUR: i64 = 1800;

/// A half hour is measured, see split, if the gateway's registers were read at least this often through it, in
/// seconds. A capture device reads them every 10 seconds.
const MAX_REGISTERS_GAP: f64 = 60.0;
/// Less of the car's or the Powerwalls' measured import than this in a half hour, in kWh, is the house's, as it is only
/// the Home Mini and the gateway being read at slightly different moments, e.g. a few Wh in a half hour without either.
const MEASUREMENT_NOISE: f64 = 0.02;

/// A site's energy registers at a moment, in Wh, which only ever go up, so that the energy between two moments is their
/// difference, as a Tesla gateway reads them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SiteRegisters {
    /// When the gateway last read its meters, as a unix timestamp in seconds.
    pub time: f64,
    /// Imported from the grid through the gateway's site meter, which the car's charger may be outside of.
    pub site_imported: f64,
    /// Exported to the grid through the site meter.
    pub site_exported: f64,
    /// Into the batteries.
    pub battery_imported: f64,
    /// Made by the solar panels.
    pub solar_exported: f64,
    /// Used by the house.
    pub load_imported: f64,
    /// The power through the site meter from the grid, into the batteries, and from the solar panels, in W, as the
    /// gateway measured it, which is steadier than the registers' changes over a few seconds, see split.
    pub site_power: f64,
    pub battery_power: f64,
    pub solar_power: f64,
}

/// What a site's devices measured, which splits the grid's import by use, see the module comment.
pub trait Measurements {
    /// What measured it, as the Electricity page shows where a period is from, e.g. Tesla.
    fn name(&self) -> String;

    /// What each of a half hour's points' import, kwh, from and to the times of bounds, with the grid meter's average
    /// power, demand, was used for, as (the car, the batteries, the house), or None if it was not measured all through
    /// the half hour.
    fn split(&self, kwh: &[f64], demand: &[Option<f64>], bounds: &[(i64, i64)]) -> Option<Vec<(f64, f64, f64)>>;
}

/// Where a module's measurements come from, e.g. the Tesla module's capture device's snapshots.
pub trait MeasurementSource {
    /// The measurements from start to end, unix timestamps.
    fn load(&self, start: i64, end: i64) -> Pin<Box<dyn Future<Output = anyhow::Result<Arc<dyn Measurements>>>>>;
}

/// Measurements from a site's energy registers, in time order, by what read them, e.g. Tesla.
pub struct RegisterMeasurements {
    pub name: String,
    pub registers: Vec<SiteRegisters>,
}

impl Measurements for RegisterMeasurements {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn split(&self, kwh: &[f64], demand: &[Option<f64>], bounds: &[(i64, i64)]) -> Option<Vec<(f64, f64, f64)>> {
        split(kwh, demand, bounds, &self.registers)
    }
}

/// The value of the register which field reads at time, between the readings either side of it, None if there is none
/// within MAX_REGISTERS_GAP either side. registers are in time order.
fn register_at(registers: &[SiteRegisters], time: f64, field: fn(&SiteRegisters) -> f64) -> Option<f64> {
    let next = registers.partition_point(|registers| registers.time < time);
    let (before, after) = (registers.get(next.checked_sub(1)?)?, registers.get(next)?);
    let span = after.time - before.time;
    if span > MAX_REGISTERS_GAP {
        return None;
    }
    Some(if span > 0.0 { field(before) + (field(after) - field(before)) * (time - before.time) / span } else { field(after) })
}

/// The energy the register which field reads counted from start to end, in kWh, see register_at.
pub(crate) fn register_kwh(registers: &[SiteRegisters], start: i64, end: i64, field: fn(&SiteRegisters) -> f64) -> Option<f64> {
    Some((register_at(registers, end as f64, field)? - register_at(registers, start as f64, field)?) / 1000.0)
}

/// Whether the gateway's registers were read all through the half hour starting at half_hour, see MAX_REGISTERS_GAP.
pub(crate) fn registers_complete(registers: &[SiteRegisters], half_hour: i64) -> bool {
    let (start, end) = (half_hour as f64, (half_hour + HALF_HOUR) as f64);
    let first = registers.partition_point(|registers| registers.time < start);
    let last = registers.partition_point(|registers| registers.time <= end);
    first > 0 && last < registers.len()
        && registers[first - 1..=last].windows(2).all(|pair| pair[1].time - pair[0].time <= MAX_REGISTERS_GAP)
}

/// How many points the car's and the Powerwalls' power is smoothed over, see smoothed.
const SMOOTHING: usize = 5;

/// Each of the values, the power of a point, as the median of it and those either side of it, SMOOTHING in all, fewer at
/// the ends, so that a load which is on for only a reading or two, which the Home Mini and the gateway, being read up to
/// 10 seconds apart, see at different moments, is not taken for the car's or the Powerwalls', which only change
/// slowly, and is the house's instead. Steps, e.g. the car starting, are kept.
fn smoothed(values: &[f64]) -> Vec<f64> {
    (0..values.len())
        .map(|index| {
            let mut window: Vec<f64> = values[index.saturating_sub(SMOOTHING / 2)..(index + SMOOTHING / 2 + 1).min(values.len())].to_vec();
            window.sort_by(f64::total_cmp);
            window[window.len() / 2]
        })
        .collect()
}

/// total shared between points in proportion to weights, or if they are all 0, to room, no point getting more than its
/// room, what does not fit shared by those with room left.
fn share(total: f64, weights: &[f64], room: &[f64]) -> Vec<f64> {
    let weights = if weights.iter().sum::<f64>() > 0.0 { weights } else { room };
    let sum: f64 = weights.iter().sum();
    if sum <= 0.0 {
        return vec![0.0; room.len()];
    }
    let mut shares: Vec<f64> = weights.iter().zip(room).map(|(weight, room)| (total * weight / sum).min(*room)).collect();
    let left_over = total - shares.iter().sum::<f64>();
    let space: f64 = room.iter().zip(&shares).map(|(room, share)| room - share).sum();
    if left_over > 1e-12 && space > 0.0 {
        for (share, room) in shares.iter_mut().zip(room) {
            *share += left_over * (room - *share) / space;
        }
    }
    shares
}

/// The average of the gateway's power readings which field reads from start to end, or if there are none, the nearest.
fn power(registers: &[SiteRegisters], start: i64, end: i64, field: fn(&SiteRegisters) -> f64) -> Option<f64> {
    let first = registers.partition_point(|registers| registers.time < start as f64);
    let last = registers.partition_point(|registers| registers.time < end as f64);
    if last > first {
        return Some(registers[first..last].iter().map(field).sum::<f64>() / (last - first) as f64);
    }
    let middle = (start + end) as f64 / 2.0;
    [first.checked_sub(1), Some(first)].into_iter().flatten()
        .filter_map(|index| registers.get(index))
        .min_by(|a, b| (a.time - middle).abs().total_cmp(&(b.time - middle).abs()))
        .map(field)
}

/// What each of a half hour's points' import, kwh, from and to the times of bounds, with the Home Mini's average power,
/// demand, was used for, as (the car, the Powerwalls, the house), from the gateway's registers, see the module comment,
/// or None if they were not read all through it. Each is exact for the half hour, from its registers' readings at its
/// start and end, and is shared between its points by the same differences of the power readings over each. Not of the
/// registers' changes over each, which are as steady, as the Powerwalls charge at a steady power, but jump about by
/// several hundred W over 10 seconds, as the Home Mini and the gateway are read at different moments.
fn split(kwh: &[f64], demand: &[Option<f64>], bounds: &[(i64, i64)], registers: &[SiteRegisters]) -> Option<Vec<(f64, f64, f64)>> {
    let half_hour = bounds.first()?.0 - bounds.first()?.0.rem_euclid(HALF_HOUR);
    if !registers_complete(registers, half_hour) {
        return None;
    }
    let site = |start, end| register_kwh(registers, start, end, |registers| registers.site_imported);
    // Into the Powerwalls from the grid, as what the solar panels made goes into them first.
    let battery = |start, end| Some((register_kwh(registers, start, end, |registers| registers.battery_imported)?
        - register_kwh(registers, start, end, |registers| registers.solar_exported)?).max(0.0));

    let total: f64 = kwh.iter().sum();
    let noise = |kwh: f64| if kwh < MEASUREMENT_NOISE { 0.0 } else { kwh };
    let ev_total = noise((total - site(half_hour, half_hour + HALF_HOUR)?).clamp(0.0, total));
    let battery_total = noise(battery(half_hour, half_hour + HALF_HOUR)?.min(total - ev_total));
    // The power of each, in W, smoothed, then times its length, as a point's share is in proportion to its energy.
    let seconds: Vec<f64> = bounds.iter().map(|(start, end)| (end - start) as f64).collect();
    let ev_power: Vec<f64> = kwh.iter().zip(demand).zip(bounds).zip(&seconds)
        .map(|(((kwh, demand), (start, end)), seconds)| {
            let home_mini = demand.unwrap_or(kwh * 3_600_000.0 / seconds.max(1.0));
            Some((home_mini - power(registers, *start, *end, |registers| registers.site_power)?).max(0.0))
        })
        .collect::<Option<_>>()?;
    let ev_weights: Vec<f64> = smoothed(&ev_power).iter().zip(&seconds).map(|(power, seconds)| power * seconds).collect();
    let ev = share(ev_total, &ev_weights, kwh);
    let battery_power: Vec<f64> = bounds.iter()
        .map(|(start, end)| Some((power(registers, *start, *end, |registers| registers.battery_power)?
            - power(registers, *start, *end, |registers| registers.solar_power)?).max(0.0)))
        .collect::<Option<_>>()?;
    let battery_weights: Vec<f64> = smoothed(&battery_power).iter().zip(&seconds).map(|(power, seconds)| power * seconds).collect();
    let room: Vec<f64> = kwh.iter().zip(&ev).map(|(kwh, ev)| (kwh - ev).max(0.0)).collect();
    let battery = share(battery_total, &battery_weights, &room);
    Some(kwh.iter().zip(ev).zip(battery).map(|((kwh, ev), battery)| (ev, battery, (kwh - ev - battery).max(0.0))).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const DAY: i64 = 1_793_232_000;

    /// The gateway's registers every 10 seconds from start up to end, with the site importing site_watts, the
    /// Powerwalls taking in battery_watts and the solar panels making nothing.
    pub(crate) fn registers(start: i64, end: i64, site_watts: f64, battery_watts: f64) -> Vec<SiteRegisters> {
        (start..=end).step_by(10)
            .map(|time| {
                let hours = (time - start) as f64 / 3600.0;
                SiteRegisters {
                    time: time as f64 + 0.3,
                    site_imported: 1000.0 + site_watts * hours,
                    site_exported: 500.0,
                    battery_imported: 2000.0 + battery_watts * hours,
                    solar_exported: 3000.0,
                    load_imported: 4000.0,
                    site_power: site_watts,
                    battery_power: battery_watts,
                    solar_power: 0.0,
                }
            })
            .collect()
    }

    #[test]
    fn the_car_is_the_home_minis_import_less_the_gateways_and_the_powerwalls_what_they_took_in() {
        // A half hour of 10 second points, the Home Mini importing 14 kW, the gateway 7 kW, of which the Powerwalls 5 kW.
        let bounds: Vec<(i64, i64)> = (0..180).map(|index| (DAY + index * 10, DAY + index * 10 + 10)).collect();
        let kwh = vec![14000.0 * 10.0 / 3_600_000.0; 180];
        let demand = vec![Some(14000.0); 180];
        let uses = split(&kwh, &demand, &bounds, &registers(DAY - 20, DAY + 1820, 7000.0, 5000.0)).unwrap();
        let total = |pick: fn(&(f64, f64, f64)) -> f64| uses.iter().map(pick).sum::<f64>();
        assert!((total(|uses| uses.0) - 3.5).abs() < 1e-6, "the car 7 kW for half an hour");
        assert!((total(|uses| uses.1) - 2.5).abs() < 1e-6, "the Powerwalls 5 kW");
        assert!((total(|uses| uses.2) - 1.0).abs() < 1e-6, "the house the rest, 2 kW");
        assert!(uses.iter().zip(&kwh).all(|(uses, kwh)| (uses.0 + uses.1 + uses.2 - kwh).abs() < 1e-9));
        // Not measured if the registers were not read all through it.
        let mut gappy = registers(DAY - 20, DAY + 1820, 7000.0, 5000.0);
        gappy.retain(|registers| !(DAY as f64 + 600.0..DAY as f64 + 700.0).contains(&registers.time));
        assert!(split(&kwh, &demand, &bounds, &gappy).is_none());
        // Shared by the steady power readings, so the same each point.
        assert!(uses.iter().all(|point| (point.0 - uses[0].0).abs() < 1e-9 && (point.1 - uses[0].1).abs() < 1e-9));
    }

    #[test]
    fn a_blip_of_a_reading_or_two_is_smoothed_away_and_a_step_is_kept() {
        assert_eq!(smoothed(&[7.0, 7.0, 9.0, 7.0, 7.0, 7.0]), [7.0; 6]);
        assert_eq!(smoothed(&[7.0, 7.0, 9.0, 5.0, 7.0, 7.0]), [7.0; 6]);
        assert_eq!(smoothed(&[0.0, 0.0, 0.0, 7.0, 7.0, 7.0]), [0.0, 0.0, 0.0, 7.0, 7.0, 7.0]);
    }
}
