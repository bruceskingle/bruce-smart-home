//! A site's topology: where in the house each module's channels measure, as a tree of circuits from the grid
//! connection down, a setting of the profile, as it differs from house to house, see docs/PowerSchema.md. E.g. in one
//! house the car's charger is outside the Tesla gateway, so the car's import is the grid's less the gateway's site
//! meter's; in another the gateway's grid clamp is at the grid connection, as installers fit it, and the car is behind
//! it. The modules only say what their channels are, see Channel, and the topology, which the user sets on the Site
//! page, splits the grid's import by use from them, see TopologySplit.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::measured::{HALF_HOUR, Measurements, Split, share, smoothed};

/// What a circuit is for, which decides what its share of the grid's import is labelled, e.g. EV Low Rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Use {
    /// The grid connection, the top of the tree.
    Grid,
    /// A board, or a gateway, which other circuits are fed from.
    Board,
    /// A car's charger.
    Car,
    /// A home battery, e.g. Powerwalls.
    Battery,
    /// Solar panels, which make energy rather than take it.
    Solar,
    /// The house's own loads.
    #[default]
    House,
}

impl Use {
    pub const ALL: [Use; 6] = [Use::Grid, Use::Board, Use::Car, Use::Battery, Use::Solar, Use::House];

    pub fn label(self) -> &'static str {
        match self {
            Use::Grid => "Grid",
            Use::Board => "Board or gateway",
            Use::Car => "Car",
            Use::Battery => "Battery",
            Use::Solar => "Solar",
            Use::House => "House",
        }
    }
}

/// A circuit of the site, with what it is for, the channels which measure it, the most detailed first, and the
/// circuits fed from it. One without channels is its parent's less its measured siblings', see TopologySplit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Circuit {
    pub name: String,
    #[serde(rename = "use")]
    pub use_: Use,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Circuit>,
}

/// One place a module measures, however many resolutions it comes in, e.g. `octopus:electricity`, the billing meter's
/// half hours and its Home Mini's readings, with what it is for the Site page, and what it usually measures, for the
/// suggested topology, e.g. the grid, or a board for a gateway's site meter.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: String,
    pub description: String,
    pub use_: Use,
}

impl Circuit {
    fn new(name: &str, use_: Use, channels: &[&str], children: Vec<Circuit>) -> Circuit {
        Circuit { name: name.to_string(), use_, channels: channels.iter().map(|channel| channel.to_string()).collect(), children }
    }

    /// A first topology for the channels the profile's modules have, to edit, from what each usually measures: the
    /// grid with whatever measures the grid connection, a gateway's site meter fed from it, if there is one, as a board
    /// feeding the batteries, the solar panels and the car, as installers fit them, and the house the rest, without
    /// the channels which measure the house, as the rest is what the others do not take. Where the car's charger
    /// really is, only the user knows, see docs/PowerSchema.md.
    pub fn suggested(channels: &[Channel]) -> Circuit {
        let ids = |use_: Use| -> Vec<&str> { channels.iter().filter(|channel| channel.use_ == use_).map(|channel| channel.id.as_str()).collect() };
        let mut fed: Vec<Circuit> = [(Use::Battery, "Battery"), (Use::Solar, "Solar"), (Use::Car, "Car")].into_iter()
            .flat_map(|(use_, name)| ids(use_).into_iter().map(move |id| Circuit::new(name, use_, &[id], vec![])))
            .collect();
        fed.push(Circuit::new("House", Use::House, &[], vec![]));
        let children = match ids(Use::Board).first() {
            Some(board) => vec![Circuit::new("Gateway", Use::Board, &[board], fed)],
            None => fed,
        };
        Circuit::new("Grid", Use::Grid, &ids(Use::Grid), children)
    }

    /// What is wrong with it, as the Site page says, nothing if it can split the import: the top is the grid, each
    /// circuit has at most one child without a channel, as the split of two can not be told, and no channel is in two
    /// places.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.use_ != Use::Grid {
            problems.push(format!("The top circuit, {}, is not the grid.", self.name));
        }
        let mut seen = BTreeSet::new();
        self.check(true, &mut seen, &mut problems);
        problems
    }

    /// The channels of it and every circuit under it.
    pub fn channel_ids(&self) -> BTreeSet<String> {
        let mut ids: BTreeSet<String> = self.channels.iter().cloned().collect();
        for child in &self.children {
            ids.extend(child.channel_ids());
        }
        ids
    }

    fn check(&self, top: bool, seen: &mut BTreeSet<String>, problems: &mut Vec<String>) {
        if !top && self.use_ == Use::Grid {
            problems.push(format!("{} is the grid, which can only be the top circuit.", self.name));
        }
        for channel in &self.channels {
            if !seen.insert(channel.clone()) {
                problems.push(format!("{channel} measures more than one circuit."));
            }
        }
        let unmeasured: Vec<&str> = self.children.iter().filter(|child| child.channels.is_empty()).map(|child| child.name.as_str()).collect();
        if unmeasured.len() > 1 {
            problems.push(format!("{} feeds {} without channels, so how its energy is shared between them can not be told.",
                self.name, unmeasured.join(" and ")));
        }
        for child in &self.children {
            child.check(false, seen, problems);
        }
    }
}

/// Less of the car's or a battery's import than this in a half hour, in kWh, is the house's, as it is only the grid
/// meter and the site's devices being read at slightly different moments, e.g. a few Wh in a half hour without either.
const MEASUREMENT_NOISE: f64 = 0.02;

/// The grid's import split by use, from measurements placed by a topology: each circuit's share of its parent's is
/// what its channel measured, a battery's less what solar panels on the same board made, as they charge it first, and
/// one without a channel has the rest, e.g. the car outside the gateway has the grid's less the gateway's. The share
/// of the car's and the batteries' circuits are the car's and the batteries', and the rest the house's.
pub struct TopologySplit {
    pub topology: Circuit,
    pub measurements: Vec<Arc<dyn Measurements>>,
}

/// The car's and the batteries' shares of a half hour, in kWh, and their power each point, in W, see TopologySplit.
#[derive(Default)]
struct Shares {
    car: f64,
    car_power: Vec<f64>,
    battery: f64,
    battery_power: Vec<f64>,
}

impl Shares {
    fn add(&mut self, use_: Use, kwh: f64, power: &[f64]) {
        let (total, sum) = match use_ {
            Use::Car => (&mut self.car, &mut self.car_power),
            Use::Battery => (&mut self.battery, &mut self.battery_power),
            _ => return,
        };
        *total += kwh;
        for (sum, power) in sum.iter_mut().zip(power) {
            *sum += power.max(0.0);
        }
    }
}

/// What a circuit measured over a half hour and through its points, see TopologySplit::measured.
struct Measured {
    flow: super::measured::Flow,
    power: Vec<f64>,
}

impl TopologySplit {
    /// What the first of a circuit's channels which is measured measured from half_hour, and its power through each
    /// of bounds, None if none of them is.
    fn measured(&self, circuit: &Circuit, half_hour: i64, bounds: &[(i64, i64)]) -> Option<Measured> {
        circuit.channels.iter().find_map(|channel| {
            self.measurements.iter().find_map(|measurements| {
                let flow = measurements.energy(channel, half_hour, half_hour + HALF_HOUR)?;
                // A channel without power readings, e.g. the gateway's load, as much all through the half hour.
                let average = (flow.import - flow.export) * 3_600_000.0 / HALF_HOUR as f64;
                let power = bounds.iter().map(|(start, end)| measurements.power(channel, *start, *end).unwrap_or(average)).collect();
                Some(Measured { flow, power })
            })
        })
    }

    /// Shares circuit's share of the import, kwh, with power its power through each point, between its children, and
    /// adds the car's and the batteries' to shares, None if it can not be split, as two children are not measured.
    fn allot(&self, circuit: &Circuit, kwh: f64, power: &[f64], half_hour: i64, bounds: &[(i64, i64)], shares: &mut Shares) -> Option<()> {
        if circuit.children.is_empty() {
            let kwh = if matches!(circuit.use_, Use::Car | Use::Battery) && kwh < MEASUREMENT_NOISE { 0.0 } else { kwh };
            shares.add(circuit.use_, kwh, power);
            return Some(());
        }
        let measured: Vec<Option<Measured>> = circuit.children.iter().map(|child| self.measured(child, half_hour, bounds)).collect();
        // What the solar panels on this board made, which goes into its batteries first.
        let mut solar = 0.0;
        let mut solar_power = vec![0.0; bounds.len()];
        for (child, measured) in circuit.children.iter().zip(&measured) {
            if let (Use::Solar, Some(measured)) = (child.use_, measured) {
                solar += measured.flow.export - measured.flow.import;
                for (sum, power) in solar_power.iter_mut().zip(&measured.power) {
                    *sum -= power;
                }
            }
        }
        // What each measured child takes from this circuit, and its power.
        let mut demands: Vec<(usize, f64, Vec<f64>)> = Vec::new();
        let mut unmeasured = None;
        for (index, (child, measured)) in circuit.children.iter().zip(&measured).enumerate() {
            match (child.use_, measured) {
                (Use::Solar, _) => {},
                (Use::Battery, Some(measured)) => {
                    let demand = (measured.flow.import - solar).max(0.0);
                    solar = (solar - measured.flow.import).max(0.0);
                    let power = measured.power.iter().zip(&solar_power).map(|(power, solar)| (power - solar).max(0.0)).collect();
                    demands.push((index, demand, power));
                },
                (_, Some(measured)) => demands.push((index, measured.flow.import, measured.power.clone())),
                (_, None) if unmeasured.is_some() => return None,
                (_, None) => unmeasured = Some(index),
            }
        }
        // The child without a channel has what the measured ones did not take, and they share the rest in proportion.
        let measured_total: f64 = demands.iter().map(|(_, demand, _)| demand).sum();
        let rest = match unmeasured {
            Some(index) => {
                let child = &circuit.children[index];
                let rest = (kwh - measured_total).clamp(0.0, kwh);
                let rest = if child.children.is_empty() && matches!(child.use_, Use::Car | Use::Battery) && rest < MEASUREMENT_NOISE { 0.0 } else { rest };
                let others: Vec<f64> = (0..bounds.len())
                    .map(|point| power[point] - circuit.children.iter().zip(&measured)
                        .filter_map(|(_, measured)| measured.as_ref().map(|measured| measured.power[point]))
                        .sum::<f64>())
                    .collect();
                self.allot(child, rest, &others, half_hour, bounds, shares)?;
                rest
            },
            None => 0.0,
        };
        let left = kwh - rest;
        for (index, demand, power) in &demands {
            let share = if measured_total > 0.0 { left * demand / measured_total } else { left / demands.len() as f64 };
            self.allot(&circuit.children[*index], share, power, half_hour, bounds, shares)?;
        }
        Some(())
    }
}

impl Split for TopologySplit {
    fn name(&self) -> String {
        self.measurements.iter().map(|measurements| measurements.name()).collect::<Vec<_>>().join(", ")
    }

    /// Each use's share of the half hour, exact from the channels' energy over it, shared between its points by their
    /// power, smoothed, rather than by their energy, which jumps about by several hundred W over 10 seconds, as the
    /// grid meter and the site's devices are read at different moments.
    fn split(&self, kwh: &[f64], demand: &[Option<f64>], bounds: &[(i64, i64)]) -> Option<Vec<(f64, f64, f64)>> {
        let half_hour = bounds.first()?.0 - bounds.first()?.0.rem_euclid(HALF_HOUR);
        // Only the measurements of the topology's channels, as e.g. a zappi's gaps do not matter if it is not placed.
        let used = self.topology.channel_ids();
        let placed: Vec<&Arc<dyn Measurements>> = self.measurements.iter()
            .filter(|measurements| measurements.channels().iter().any(|channel| used.contains(channel)))
            .collect();
        if placed.is_empty() || !placed.iter().all(|measurements| measurements.complete(half_hour)) {
            return None;
        }
        let seconds: Vec<f64> = bounds.iter().map(|(start, end)| (end - start) as f64).collect();
        let grid_power: Vec<f64> = kwh.iter().zip(demand).zip(&seconds)
            .map(|((kwh, demand), seconds)| demand.unwrap_or(kwh * 3_600_000.0 / seconds.max(1.0)))
            .collect();
        let mut shares = Shares { car_power: vec![0.0; kwh.len()], battery_power: vec![0.0; kwh.len()], ..Shares::default() };
        let total: f64 = kwh.iter().sum();
        self.allot(&self.topology, total, &grid_power, half_hour, bounds, &mut shares)?;

        let weights = |power: &[f64]| -> Vec<f64> { smoothed(power).iter().zip(&seconds).map(|(power, seconds)| power * seconds).collect() };
        let car = share(shares.car.min(total), &weights(&shares.car_power), kwh);
        let room: Vec<f64> = kwh.iter().zip(&car).map(|(kwh, car)| (kwh - car).max(0.0)).collect();
        let battery = share(shares.battery.min(total - shares.car.min(total)), &weights(&shares.battery_power), &room);
        Some(kwh.iter().zip(car).zip(battery).map(|((kwh, car), battery)| (car, battery, (kwh - car - battery).max(0.0))).collect())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::power::measured::{RegisterMeasurements, SITE_CHANNEL, BATTERY_CHANNEL, SOLAR_CHANNEL};
    use crate::power::measured::tests::registers;

    const DAY: i64 = 1_793_232_000;

    /// The first household's topology: the car's charger beside the Tesla gateway, outside its site meter.
    pub(crate) fn beside_the_gateway() -> Circuit {
        Circuit::new("Grid", Use::Grid, &["octopus:electricity"], vec![
            Circuit::new("Car", Use::Car, &[], vec![]),
            Circuit::new("Gateway", Use::Board, &[SITE_CHANNEL], vec![
                Circuit::new("Battery", Use::Battery, &[BATTERY_CHANNEL], vec![]),
                Circuit::new("Solar", Use::Solar, &[SOLAR_CHANNEL], vec![]),
                Circuit::new("House", Use::House, &[], vec![]),
            ]),
        ])
    }

    fn tesla(registers: Vec<crate::power::measured::SiteRegisters>) -> Vec<Arc<dyn Measurements>> {
        vec![Arc::new(RegisterMeasurements { name: String::from("Tesla"), registers })]
    }

    /// A half hour of 10 second points, the grid importing 14 kW.
    fn half_hour() -> (Vec<(i64, i64)>, Vec<f64>, Vec<Option<f64>>) {
        let bounds: Vec<(i64, i64)> = (0..180).map(|index| (DAY + index * 10, DAY + index * 10 + 10)).collect();
        (bounds, vec![14000.0 * 10.0 / 3_600_000.0; 180], vec![Some(14000.0); 180])
    }

    fn totals(uses: &[(f64, f64, f64)]) -> (f64, f64, f64) {
        uses.iter().fold((0.0, 0.0, 0.0), |sum, uses| (sum.0 + uses.0, sum.1 + uses.1, sum.2 + uses.2))
    }

    #[test]
    fn beside_the_gateway_the_car_is_the_grids_import_less_the_gateways() {
        // The gateway importing 7 kW, of which the Powerwalls 5 kW.
        let (bounds, kwh, demand) = half_hour();
        let measurements = tesla(registers(DAY - 20, DAY + 1820, 7000.0, 5000.0));
        let topology = beside_the_gateway();
        let split = TopologySplit { topology: topology.clone(), measurements };
        let uses = split.split(&kwh, &demand, &bounds).unwrap();
        let (car, battery, house) = totals(&uses);
        assert!((car - 3.5).abs() < 1e-6, "the car 7 kW for half an hour, {car}");
        assert!((battery - 2.5).abs() < 1e-6, "the Powerwalls 5 kW, {battery}");
        assert!((house - 1.0).abs() < 1e-6, "the house the rest, 2 kW, {house}");
        assert!(uses.iter().zip(&kwh).all(|(uses, kwh)| (uses.0 + uses.1 + uses.2 - kwh).abs() < 1e-9));
        // Shared by the steady power readings, so the same each point.
        assert!(uses.iter().all(|point| (point.0 - uses[0].0).abs() < 1e-9 && (point.1 - uses[0].1).abs() < 1e-9));
        // Not measured if the registers were not read all through it.
        let mut gappy = registers(DAY - 20, DAY + 1820, 7000.0, 5000.0);
        gappy.retain(|registers| !(DAY as f64 + 600.0..DAY as f64 + 700.0).contains(&registers.time));
        let gappy = tesla(gappy);
        assert!(TopologySplit { topology, measurements: gappy }.split(&kwh, &demand, &bounds).is_none());
    }

    #[test]
    fn with_the_gateway_at_the_grid_connection_the_car_is_the_houses_without_a_channel_of_its_own() {
        // As installers fit it: the gateway's clamp measures the whole grid connection, so without the zappi's own
        // channel the car can not be told from the house.
        let (bounds, kwh, demand) = half_hour();
        let measurements = tesla(registers(DAY - 20, DAY + 1820, 14000.0, 5000.0));
        let topology = Circuit::new("Grid", Use::Grid, &["octopus:electricity", SITE_CHANNEL], vec![
            Circuit::new("Battery", Use::Battery, &[BATTERY_CHANNEL], vec![]),
            Circuit::new("Solar", Use::Solar, &[SOLAR_CHANNEL], vec![]),
            Circuit::new("House", Use::House, &[], vec![]),
        ]);
        let split = TopologySplit { topology, measurements };
        let (car, battery, house) = totals(&split.split(&kwh, &demand, &bounds).unwrap());
        assert!(car.abs() < 1e-9 && (battery - 2.5).abs() < 1e-6 && (house - 4.5).abs() < 1e-6, "{car} {battery} {house}");
    }

    #[test]
    fn two_circuits_without_channels_on_one_board_can_not_be_split() {
        let (bounds, kwh, demand) = half_hour();
        let measurements = tesla(registers(DAY - 20, DAY + 1820, 7000.0, 5000.0));
        let mut topology = beside_the_gateway();
        topology.children[1].channels.clear();
        assert!(TopologySplit { topology: topology.clone(), measurements }.split(&kwh, &demand, &bounds).is_none());
        assert_eq!(topology.problems().len(), 1, "{:?}", topology.problems());
        assert!(beside_the_gateway().problems().is_empty());
    }

    #[test]
    fn the_suggestion_puts_the_gateway_under_the_grid_and_the_car_behind_it_as_installers_do() {
        let channel = |id: &str, use_| Channel { id: id.to_string(), description: String::new(), use_ };
        let mut channels = vec![
            channel("octopus:electricity", Use::Grid),
            channel(SITE_CHANNEL, Use::Board),
            channel(BATTERY_CHANNEL, Use::Battery),
            channel(SOLAR_CHANNEL, Use::Solar),
            channel("tesla:load", Use::House),
        ];
        let mut expected = beside_the_gateway();
        expected.children.remove(0);
        assert_eq!(Circuit::suggested(&channels), expected);
        channels.extend([channel("myenergi:Z1", Use::Car), channel("myenergi:Z1:grid", Use::Grid)]);
        expected.channels.push(String::from("myenergi:Z1:grid"));
        expected.children[0].children.insert(2, Circuit::new("Car", Use::Car, &["myenergi:Z1"], vec![]));
        assert_eq!(Circuit::suggested(&channels), expected);
    }

    #[test]
    fn a_measured_car_beside_the_gateway_shares_the_grids_import_with_it_in_proportion() {
        // The zappi's own channel says 3.6 kWh, the gateway 3.5 kWh, of a grid import of 7 kWh, so each is scaled to
        // fit, and a measurement not in the topology, with gaps, does not stop the split.
        let (bounds, kwh, demand) = half_hour();
        let zappi = Arc::new(Fixed { channel: "myenergi:Z1", kwh: 3.6, watts: 7200.0 });
        let unplaced = Arc::new(Fixed { channel: "other:x", kwh: 0.0, watts: 0.0 });
        let mut measurements = tesla(registers(DAY - 20, DAY + 1820, 7000.0, 5000.0));
        measurements.push(zappi);
        measurements.push(unplaced);
        let mut topology = beside_the_gateway();
        topology.children[0].channels.push(String::from("myenergi:Z1"));
        let split = TopologySplit { topology, measurements };
        let (car, battery, house) = totals(&split.split(&kwh, &demand, &bounds).unwrap());
        let car_share = 7.0 * 3.6 / 7.1;
        assert!((car - car_share).abs() < 1e-6, "{car}");
        assert!((battery - 2.5).abs() < 1e-6 && (house - (7.0 - car_share - 2.5)).abs() < 1e-6, "{battery} {house}");
    }

    /// A channel at a steady power all the time, or with other:x, never complete.
    struct Fixed {
        channel: &'static str,
        kwh: f64,
        watts: f64,
    }

    impl Measurements for Fixed {
        fn name(&self) -> String {
            String::from("Fixed")
        }

        fn channels(&self) -> Vec<String> {
            vec![self.channel.to_string()]
        }

        fn complete(&self, _half_hour: i64) -> bool {
            self.channel != "other:x"
        }

        fn energy(&self, channel: &str, _start: i64, _end: i64) -> Option<crate::power::measured::Flow> {
            (channel == self.channel).then_some(crate::power::measured::Flow { import: self.kwh, export: 0.0 })
        }

        fn power(&self, channel: &str, _start: i64, _end: i64) -> Option<f64> {
            (channel == self.channel).then_some(self.watts)
        }
    }

    #[test]
    fn it_is_saved_as_json_with_use_and_no_empty_lists() {
        let json = serde_json::to_value(Circuit::new("House", Use::House, &[], vec![])).unwrap();
        assert_eq!(json, serde_json::json!({"name": "House", "use": "house"}));
    }
}
