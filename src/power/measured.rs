//! What a site's own devices measured, e.g. a Tesla gateway's energy registers, as channels, e.g. `tesla:site`, which
//! the site's topology places, see power::topology, to split the grid's import by what it was used for, see Split,
//! without the modules knowing where their channels are.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub(crate) const HALF_HOUR: i64 = 1800;

/// A half hour is measured, see power::topology, if the gateway's registers were read at least this often through it, in
/// seconds. A capture device reads them every 10 seconds.
const MAX_REGISTERS_GAP: f64 = 60.0;

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
    /// Out of the batteries.
    pub battery_exported: f64,
    /// Made by the solar panels.
    pub solar_exported: f64,
    /// Used by the house.
    pub load_imported: f64,
    /// The power through the site meter from the grid, into the batteries, and from the solar panels, in W, as the
    /// gateway measured it, which is steadier than the registers' changes over a few seconds, see power::topology.
    pub site_power: f64,
    pub battery_power: f64,
    pub solar_power: f64,
}

/// A channel's energy over a span of time, in kWh, into its circuit, import, and out of it, export, e.g. a battery's
/// charge and discharge, or what solar panels made.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flow {
    pub import: f64,
    pub export: f64,
}

/// What a module's devices measured, e.g. the Tesla gateway's meters, as channels, each one place in the site's
/// topology, see power::topology.
pub trait Measurements {
    /// What measured it, as the Electricity page shows where a period is from, e.g. Tesla.
    fn name(&self) -> String;

    /// The channels it measures, e.g. `tesla:site`.
    fn channels(&self) -> Vec<String>;

    /// Whether its channels were measured all through the half hour starting at half_hour.
    fn complete(&self, half_hour: i64) -> bool;

    /// A channel's energy from start to end, None if it is not one of its channels or was not measured then.
    fn energy(&self, channel: &str, start: i64, end: i64) -> Option<Flow>;

    /// A channel's average power from start to end, in W, into its circuit, negative out of it, e.g. while solar
    /// panels make, which is steadier than its energy's changes over a few seconds, see power::topology.
    fn power(&self, channel: &str, start: i64, end: i64) -> Option<f64>;

    /// The time of its latest reading, as a unix timestamp, None if it has none, for the Live page, see power::live.
    fn latest(&self) -> Option<i64>;
}

/// What splits the grid's import by what it was used for, which combine is given without knowing how, see
/// power::topology::TopologySplit.
pub trait Split {
    /// What measured it, as the Electricity page shows where a period is from, e.g. Tesla.
    fn name(&self) -> String;

    /// What each of a half hour's points' import, kwh, from and to the times of bounds, with the grid meter's average
    /// power, demand, was used for, in kWh by category, e.g. Car, Battery and House, see power::topology, or None if
    /// it was not measured all through the half hour.
    fn split(&self, kwh: &[f64], demand: &[Option<f64>], bounds: &[(i64, i64)]) -> Option<Vec<std::collections::BTreeMap<String, f64>>>;
}

/// Where a module's measurements come from, e.g. the Tesla module's capture device's snapshots.
pub trait MeasurementSource {
    /// The measurements from start to end, unix timestamps.
    fn load(&self, start: i64, end: i64) -> Pin<Box<dyn Future<Output = anyhow::Result<Arc<dyn Measurements>>>>>;
}

/// The Tesla gateway's meters' channels, see RegisterMeasurements.
pub const SITE_CHANNEL: &str = "tesla:site";
pub const BATTERY_CHANNEL: &str = "tesla:battery";
pub const SOLAR_CHANNEL: &str = "tesla:solar";
pub const LOAD_CHANNEL: &str = "tesla:load";

/// Measurements from a site's energy registers, in time order, by what read them, e.g. Tesla, whose channels are the
/// gateway's meters: the site meter, the batteries, the solar panels, and the load, which is the house's as the
/// gateway works it out.
pub struct RegisterMeasurements {
    pub name: String,
    pub registers: Vec<SiteRegisters>,
}

/// A register's fields of a channel: its import's, its export's, and its power into its circuit, if it has one.
type Fields = (fn(&SiteRegisters) -> f64, fn(&SiteRegisters) -> f64, Option<fn(&SiteRegisters) -> f64>);

fn channel_fields(channel: &str) -> Option<Fields> {
    Some(match channel {
        SITE_CHANNEL => (|registers| registers.site_imported, |registers| registers.site_exported, Some(|registers| registers.site_power)),
        BATTERY_CHANNEL => (|registers| registers.battery_imported, |registers| registers.battery_exported, Some(|registers| registers.battery_power)),
        SOLAR_CHANNEL => (|_| 0.0, |registers| registers.solar_exported, Some(|registers| -registers.solar_power)),
        LOAD_CHANNEL => (|registers| registers.load_imported, |_| 0.0, None),
        _ => return None,
    })
}

impl Measurements for RegisterMeasurements {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn channels(&self) -> Vec<String> {
        [SITE_CHANNEL, BATTERY_CHANNEL, SOLAR_CHANNEL, LOAD_CHANNEL].map(String::from).to_vec()
    }

    fn complete(&self, half_hour: i64) -> bool {
        registers_complete(&self.registers, half_hour)
    }

    fn energy(&self, channel: &str, start: i64, end: i64) -> Option<Flow> {
        let (import, export, _) = channel_fields(channel)?;
        Some(Flow { import: register_kwh(&self.registers, start, end, import)?, export: register_kwh(&self.registers, start, end, export)? })
    }

    fn power(&self, channel: &str, start: i64, end: i64) -> Option<f64> {
        power(&self.registers, start, end, channel_fields(channel)?.2?)
    }

    fn latest(&self) -> Option<i64> {
        self.registers.last().map(|registers| registers.time as i64)
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
pub(crate) fn smoothed(values: &[f64]) -> Vec<f64> {
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
pub(crate) fn share(total: f64, weights: &[f64], room: &[f64]) -> Vec<f64> {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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
                    battery_exported: 100.0,
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
    fn a_blip_of_a_reading_or_two_is_smoothed_away_and_a_step_is_kept() {
        assert_eq!(smoothed(&[7.0, 7.0, 9.0, 7.0, 7.0, 7.0]), [7.0; 6]);
        assert_eq!(smoothed(&[7.0, 7.0, 9.0, 5.0, 7.0, 7.0]), [7.0; 6]);
        assert_eq!(smoothed(&[0.0, 0.0, 0.0, 7.0, 7.0, 7.0]), [0.0, 0.0, 0.0, 7.0, 7.0, 7.0]);
    }
}
