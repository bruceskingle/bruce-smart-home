//! What each circuit of a site's topology is taking or giving now, for Home's Live page: its power over the latest
//! WINDOW which any measurement placed in the topology has, see power::topology, from its channel, or for the one
//! circuit of a board without a channel, the board's less its measured circuits', and for the grid, if no channel which
//! the measurements have measures it, the total of the circuits it feeds. A channel whose measurements have not reached
//! that time yet, e.g. a zappi's, whose records are of minutes, fetched from myenergi every few minutes, has its own
//! latest WINDOW's, and says when that was, see Live::older, as does what is worked out from it.

use std::sync::Arc;

use super::measured::Measurements;
use super::topology::{Circuit, Use};

/// How long the power is the average of, as the zappi's records are of minutes.
pub const WINDOW: i64 = 60;

/// Less than this from solar panels, in W, is none, as their meter reads a few W either way at night, from its clamp's
/// offset and the inverter's own draw. It is taken as none before the rest is worked out, e.g. the house's, so that what
/// is shown still adds up.
pub const SOLAR_NOISE: f64 = 50.0;

/// Each circuit's power, in W, into it, negative out of it, e.g. solar panels making or a battery discharging, in the
/// order of a walk of the topology, each circuit before those it feeds, None where it can not be worked out, and the
/// time it is of.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub time: i64,
    pub power: Vec<Option<f64>>,
    /// For each circuit, in the same order, the time its power is of, if that is before time, e.g. the zappi's latest
    /// minute, or for one worked out from it, the oldest of those it is worked out from.
    pub older: Vec<Option<i64>>,
}

/// A circuit's power, and the time it is of if that is older than Live::time.
type Reading = (f64, Option<i64>);

/// The circuits' power at the latest time any placed measurement has, see the module comment, None if there is none,
/// e.g. before the first readings.
pub fn live(topology: &Circuit, measurements: &[Arc<dyn Measurements>]) -> Option<Live> {
    let used = topology.channel_ids();
    let placed: Vec<&Arc<dyn Measurements>> = measurements.iter()
        .filter(|measurements| measurements.channels().iter().any(|channel| used.contains(channel)))
        .collect();
    let time = placed.iter().filter_map(|measurements| measurements.latest()).max()?;
    let measured = |circuit: &Circuit| -> Option<Reading> {
        let quiet = |(power, older): Reading| (if circuit.use_ == Use::Solar && power.abs() < SOLAR_NOISE { 0.0 } else { power }, older);
        circuit.channels.iter().find_map(|channel| {
            placed.iter().find_map(|measurements| {
                if let Some(power) = measurements.power(channel, time - WINDOW, time) {
                    return Some((power, None));
                }
                // Not there yet: its own latest.
                let latest = measurements.latest()?;
                measurements.power(channel, latest - WINDOW, latest).map(|power| (power, Some(latest)))
            })
        }).map(quiet)
    };
    let (mut power, mut older) = (Vec::new(), Vec::new());
    walk(topology, measured(topology), &measured, &mut power, &mut older);
    Some(Live { time, power, older })
}

/// The oldest of the times readings are of, see Live::older.
fn oldest<'a>(readings: impl IntoIterator<Item = &'a Reading>) -> Option<i64> {
    readings.into_iter().filter_map(|(_, older)| *older).min()
}

/// Adds circuit's power, own, if it is known, or if it is not, the total of the circuits it feeds, if they are all
/// known, then theirs, to power and older, see Live.
fn walk(circuit: &Circuit, own: Option<Reading>, measured: &dyn Fn(&Circuit) -> Option<Reading>, power: &mut Vec<Option<f64>>,
    older: &mut Vec<Option<i64>>)
{
    let children: Vec<Option<Reading>> = circuit.children.iter().map(measured).collect();
    let unmeasured = children.iter().filter(|child| child.is_none()).count();
    let known: f64 = children.iter().flatten().map(|(power, _)| power).sum();
    let known_older = oldest(children.iter().flatten());
    let own = own.or_else(|| (unmeasured == 0 && !children.is_empty()).then_some((known, known_older)));
    power.push(own.map(|(power, _)| power));
    older.push(own.and_then(|(_, older)| older));
    for (child, child_reading) in circuit.children.iter().zip(children) {
        // The one without a channel has the rest.
        let child_reading = child_reading.or_else(|| {
            own.filter(|_| unmeasured == 1).map(|(own_power, own_older)| {
                (own_power - known, [own_older, known_older].into_iter().flatten().min())
            })
        });
        walk(child, child_reading, measured, power, older);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::measured::{RegisterMeasurements, tests::registers};
    use crate::power::topology::tests::beside_the_gateway;

    const DAY: i64 = 1_793_232_000;

    #[test]
    fn the_gateways_meters_give_the_house_and_the_grid_is_unknown_without_the_car() {
        // The gateway importing 7 kW, of which the Powerwalls 5 kW, so the house 2 kW; the car beside it has no
        // channel, and nothing measures the grid, so neither can be worked out.
        let measurements: Vec<Arc<dyn Measurements>> = vec![Arc::new(RegisterMeasurements {
            name: String::from("Tesla"),
            registers: registers(DAY - 20, DAY + 1820, 7000.0, 5000.0),
        })];
        let live = live(&beside_the_gateway(), &measurements).unwrap();
        assert_eq!(live.time, DAY + 1820);
        // Grid, Car, Gateway, Battery, Solar, House.
        let power: Vec<Option<i64>> = live.power.iter().map(|power| power.map(|power| power.round() as i64)).collect();
        assert_eq!(power, [None, None, Some(7000), Some(5000), Some(0), Some(2000)]);
        assert!(live.older.iter().all(Option::is_none));
    }

    #[test]
    fn solar_noise_is_none_and_the_house_has_it_so_they_still_add_up() {
        // 20 W from the solar panels at night, which is noise.
        let mut night = registers(DAY - 20, DAY + 1820, 7000.0, 5000.0);
        for registers in &mut night {
            registers.solar_power = 20.0;
        }
        let measurements: Vec<Arc<dyn Measurements>> = vec![Arc::new(RegisterMeasurements { name: String::from("Tesla"), registers: night })];
        let live = live(&beside_the_gateway(), &measurements).unwrap();
        let power: Vec<Option<i64>> = live.power.iter().map(|power| power.map(|power| power.round() as i64)).collect();
        // Gateway, Battery, Solar, House: the house is the gateway's less the batteries', with the solar none.
        assert_eq!(power[2..], [Some(7000), Some(5000), Some(0), Some(2000)]);
    }

    /// A car's charger whose readings stop at latest, at a steady power, as a zappi's minutes, fetched every few minutes.
    struct Lagging {
        latest: i64,
        watts: f64,
    }

    impl Measurements for Lagging {
        fn name(&self) -> String {
            String::from("zappi")
        }

        fn channels(&self) -> Vec<String> {
            vec![String::from("myenergi:Z1")]
        }

        fn complete(&self, _half_hour: i64) -> bool {
            true
        }

        fn energy(&self, _channel: &str, _start: i64, _end: i64) -> Option<crate::power::measured::Flow> {
            None
        }

        fn power(&self, channel: &str, _start: i64, end: i64) -> Option<f64> {
            (channel == "myenergi:Z1" && end <= self.latest).then_some(self.watts)
        }

        fn latest(&self) -> Option<i64> {
            Some(self.latest)
        }
    }

    #[test]
    fn a_channel_behind_the_others_gives_its_latest_and_says_when() {
        let measurements: Vec<Arc<dyn Measurements>> = vec![
            Arc::new(RegisterMeasurements { name: String::from("Tesla"), registers: registers(DAY - 20, DAY + 1820, 7000.0, 5000.0) }),
            Arc::new(Lagging { latest: DAY + 1500, watts: 7200.0 }),
        ];
        let mut topology = beside_the_gateway();
        topology.children[0].channels.push(String::from("myenergi:Z1"));
        let live = live(&topology, &measurements).unwrap();
        // The gateway's latest, the newest.
        assert_eq!(live.time, DAY + 1820);
        let power: Vec<Option<i64>> = live.power.iter().map(|power| power.map(|power| power.round() as i64)).collect();
        assert_eq!(power, [Some(14200), Some(7200), Some(7000), Some(5000), Some(0), Some(2000)]);
        // The car, and the grid worked out from it, are of the car's latest; the gateway's are current.
        assert_eq!(live.older, [Some(DAY + 1500), Some(DAY + 1500), None, None, None, None]);
    }
}
