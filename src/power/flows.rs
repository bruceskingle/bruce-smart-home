//! Where a site's energy came from and where it went, in each period, for the Electricity page's Flows view: from the
//! grid, the solar panels and the batteries discharging, to each category of load, e.g. the car and the house, the
//! batteries charging and export, as the site's topology places the measurements, see power::topology.
//!
//! Each of the grid's readings, e.g. the Home Mini's of a minute, or a half hour's, is worked out as the checks are, see
//! power::check: each circuit's net import over it, what went into it less what came out, is its channel's, or for the
//! one without a channel, its parent's less its measured siblings', and energy is conserved, so a load's net import is
//! what it used. Periods longer than the readings add them up. A reading in a half hour the measurements do not cover
//! all of, e.g. before the capture devices, has only the grid's import and export, its import as NOT_MEASURED.

use std::collections::BTreeMap;
use std::sync::Arc;

use time_tz::Tz;

use super::measured::{Flow, HALF_HOUR, Measurements};
use super::topology::{Circuit, Use, label_prefix};
use crate::period::Period;

/// The sources, as the graph labels them, and the uses which are not loads, see topology::label_prefix for the loads'.
pub const GRID: &str = "Grid";
pub const SOLAR: &str = "Solar";
pub const BATTERY: &str = "Battery";
pub const NOT_MEASURED: &str = "Not measured";

/// Their colours, see crate::known_labels: the grid grey-blue, the solar panels yellow, as on the Site page.
pub const LABELS: [(&str, &str); 3] = [(GRID, "#8a9bb0"), (SOLAR, "#f2c94c"), (NOT_MEASURED, "#5a5a66")];

/// A period's energy, in kWh, by where it came from and where it went, e.g. Grid and Solar, and Home, EV and Export.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowPoint {
    pub start: i64,
    pub end: i64,
    pub sources: BTreeMap<String, f64>,
    pub uses: BTreeMap<String, f64>,
}

impl FlowPoint {
    fn add(&mut self, other: &FlowPoint) {
        for (label, kwh) in &other.sources {
            *self.sources.entry(label.clone()).or_default() += kwh;
        }
        for (label, kwh) in &other.uses {
            *self.uses.entry(label.clone()).or_default() += kwh;
        }
    }
}

/// A reading of the grid, from start to end, as unix timestamps, with its import and export, in kWh, as the grid meter
/// or the supplier's meters measured it, see power::electricity, e.g. a minute of the Home Mini's, or a half hour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridReading {
    pub start: i64,
    pub end: i64,
    pub import: f64,
    pub export: f64,
}

/// The flows of each period of period which grid has readings in, in time order: each reading's, if it is no longer
/// than a half hour, as the readings are of the period then, otherwise their total over each period.
pub fn flows(topology: &Circuit, measurements: &[Arc<dyn Measurements>], grid: &[GridReading], period: Period, tz: &'static Tz) -> Vec<FlowPoint> {
    let used = topology.channel_ids();
    let placed: Vec<&Arc<dyn Measurements>> = measurements.iter()
        .filter(|measurements| measurements.channels().iter().any(|channel| used.contains(channel)))
        .collect();
    let mut periods: BTreeMap<i64, FlowPoint> = BTreeMap::new();
    for reading in grid {
        // Measured all through the half hour it is in, as the measurements say of half hours.
        let half_hour = reading.start - reading.start.rem_euclid(HALF_HOUR);
        let complete = !placed.is_empty() && placed.iter().all(|measurements| measurements.complete(half_hour));
        let point = complete.then(|| reading_flows(topology, &placed, reading)).flatten()
            .unwrap_or_else(|| FlowPoint {
                sources: BTreeMap::from([(String::from(GRID), reading.import)]),
                uses: BTreeMap::from([(String::from(NOT_MEASURED), reading.import), (String::from(super::EXPORT_LABEL), reading.export)]),
                ..FlowPoint::default()
            });
        let (start, end) = if period > Period::HalfHour {
            crate::period::period_bounds(reading.start, period, tz)
        } else {
            (reading.start, reading.end)
        };
        periods.entry(start).or_insert_with(|| FlowPoint { start, end, ..FlowPoint::default() }).add(&point);
    }
    periods.into_values().collect()
}

/// What a circuit's first channel which has the span measured of it from start to end, None if none has.
fn measured(circuit: &Circuit, measurements: &[&Arc<dyn Measurements>], start: i64, end: i64) -> Option<Flow> {
    circuit.channels.iter().find_map(|channel| {
        measurements.iter().find_map(|measurements| measurements.energy(channel, start, end))
    })
}

/// A reading's flows, see the module comment, None if the topology can not place them, e.g. two of a circuit's
/// circuits are without a channel.
fn reading_flows(topology: &Circuit, measurements: &[&Arc<dyn Measurements>], reading: &GridReading) -> Option<FlowPoint> {
    let mut point = FlowPoint::default();
    point.sources.insert(String::from(GRID), reading.import);
    point.uses.insert(String::from(super::EXPORT_LABEL), reading.export);
    let root = Flow { import: reading.import, export: reading.export };
    walk(topology, root, measurements, (reading.start, reading.end), &mut point)?;
    Some(point)
}

/// Adds what circuit, whose energy is flow, gave and took to point, and that of the circuits it feeds, see
/// reading_flows, over span, its start and end.
fn walk(circuit: &Circuit, flow: Flow, measurements: &[&Arc<dyn Measurements>], span: (i64, i64), point: &mut FlowPoint) -> Option<()> {
    let net = flow.import - flow.export;
    if circuit.children.is_empty() {
        let add = |map: &mut BTreeMap<String, f64>, label: &str, kwh: f64| {
            if kwh > 0.0 {
                *map.entry(label.to_string()).or_default() += kwh;
            }
        };
        match &circuit.use_ {
            Use::Battery => {
                add(&mut point.sources, BATTERY, flow.export);
                add(&mut point.uses, BATTERY, flow.import);
            },
            Use::Solar => add(&mut point.sources, SOLAR, -net),
            Use::Load(category) => add(&mut point.uses, label_prefix(category), net),
            // A board which feeds nothing, or the grid alone, whose energy went on something not known.
            _ => add(&mut point.uses, label_prefix(super::topology::UNKNOWN), net),
        }
        return Some(());
    }
    let children: Vec<Option<Flow>> = circuit.children.iter().map(|child| measured(child, measurements, span.0, span.1)).collect();
    if children.iter().filter(|child| child.is_none()).count() > 1 {
        return None;
    }
    let measured_net: f64 = children.iter().flatten().map(|child| child.import - child.export).sum();
    for (child, child_flow) in circuit.children.iter().zip(children) {
        // The rest, whose import and export are not known apart, only its net.
        let child_flow = child_flow.unwrap_or_else(|| {
            let rest = net - measured_net;
            Flow { import: rest.max(0.0), export: (-rest).max(0.0) }
        });
        walk(child, child_flow, measurements, span, point)?;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::measured::{RegisterMeasurements, tests::registers};
    use crate::power::topology::tests::beside_the_gateway;

    const DAY: i64 = 1_793_232_000;

    #[test]
    fn the_grids_import_goes_to_the_car_the_powerwalls_and_the_house() {
        // The grid importing 14 kW for half an hour, the gateway 7 kW of it, of which the Powerwalls 5 kW.
        let measurements: Vec<Arc<dyn Measurements>> = vec![Arc::new(RegisterMeasurements {
            name: String::from("Tesla"),
            registers: registers(DAY - 20, DAY + 1820, 7000.0, 5000.0),
        })];
        let reading = |start: i64, end: i64, import: f64, export: f64| GridReading { start, end, import, export };
        let grid = [reading(DAY, DAY + HALF_HOUR, 7.0, 0.0), reading(DAY + HALF_HOUR, DAY + 2 * HALF_HOUR, 1.0, 0.2)];
        let points = flows(&beside_the_gateway(), &measurements, &grid, Period::HalfHour, time_tz::timezones::db::UTC);
        assert_eq!(points.len(), 2);
        let near = |map: &BTreeMap<String, f64>, label: &str, kwh: f64| (map.get(label).copied().unwrap_or_default() - kwh).abs() < 1e-6;
        let first = &points[0];
        assert!(near(&first.sources, GRID, 7.0), "{first:?}");
        assert!(near(&first.uses, "EV", 3.5) && near(&first.uses, BATTERY, 2.5) && near(&first.uses, "Home", 1.0), "{first:?}");
        // The registers stop, so the second is not measured: the grid's import, as not measured, and its export.
        let second = &points[1];
        assert!(near(&second.uses, NOT_MEASURED, 1.0) && near(&second.uses, crate::power::EXPORT_LABEL, 0.2), "{second:?}");
        // A day adds them up.
        let days = flows(&beside_the_gateway(), &measurements, &grid, Period::Day, time_tz::timezones::db::UTC);
        assert_eq!(days.len(), 1);
        assert!(near(&days[0].sources, GRID, 8.0), "{days:?}");
        // A minute of it, from the gateway's registers over the minute.
        let minute = flows(&beside_the_gateway(), &measurements, &[reading(DAY + 600, DAY + 660, 14.0 / 60.0, 0.0)], Period::Minute,
            time_tz::timezones::db::UTC);
        assert_eq!((minute[0].start, minute[0].end), (DAY + 600, DAY + 660));
        assert!(near(&minute[0].uses, "EV", 7.0 / 60.0) && near(&minute[0].uses, "Home", 2.0 / 60.0), "{minute:?}");
    }
}
