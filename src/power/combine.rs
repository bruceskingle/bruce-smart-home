//! Electricity imported and exported, combined from all the readings the profile's modules give, see SiteData: a
//! supplier's meters' readings, e.g. Octopus's half hours, which split the import into the rates it is billed at, e.g.
//! Home Low Rate, but arrive a few days later, and a grid meter's readings every 10 seconds, e.g. the Octopus Home
//! Mini's, which are up to date but not split, and only go back as far as they were saved. Written for Octopus's
//! readings first, so it describes them, but no module's in particular.
//!
//! The energy of each half hour is from the Home Mini if it has all of the half hour's readings, as it has the detail
//! within the half hour, otherwise from the meters. The two agree to within 0.1% over a half hour. The Home Mini's
//! import is split into the labels of the import meter's reading for the half hour, in its proportions. That is exact
//! for the rates. Within the half hour the share the supplier billed as the car's, see MeterReading::uses, is placed as
//! a block at the charger's power, as the car charges at a constant power, and the rest is the house's, see
//! split_half_hour. The
//! car can not be told apart from the house's other loads by the power alone, e.g. a home battery charging at night.
//!
//! Import in a half hour which the meter has no reading for yet, e.g. today, is estimated to be at the rate the supplier's
//! module says, from its tariff's rules and whether the car was charging, see Tariff, which no other code knows. The
//! car's share is not estimated. If the rate can not be estimated the import is UNMETERED_LABEL.
//!
//! Where the site's devices measured a half hour too, e.g. a capture device reading a Tesla gateway's registers every few
//! seconds, the half hour's import is split by what it was used for instead, by the site's topology, see
//! power::topology, which combine is given without knowing how: e.g. where the zappi is outside the gateway's meter the
//! car's is the Home Mini's import less the gateway's, the Powerwalls' what they took in less what the solar panels
//! made, and the rest the house's, each in the rates of the meter's labels. That is
//! measured, unlike the meter's EV labels, which Octopus models, e.g. as the same energy every half hour of a charge,
//! which is kept for each point to compare, see ElectricityPoint::ev_comparison. Over a night of charging the car's was
//! within 0.2% of the zappi's own total.
//!
//! The supplier's bill is a fact, and the app only says who used what it billed: the import's kWh at each rate in a
//! half hour, and their cost, are the meter's. A supplier's rate may depend on what it took the energy to be for, e.g.
//! Octopus bills the car's charging at the peak rate past a limit, even in the cheap times, while the house's is cheap,
//! so the kWh it billed as a use go to that use as far as it was measured to use them, and only the rest to the other
//! uses, at the rates they were billed at, see billed_uses. The rate and use of each of the meter's labels are the
//! module's to say, see MeterReading::rates and uses.
//!
//! The thresholds for seeing the car charging, CHARGING_POWER_SHARE and CHARGING_READINGS, were chosen from one
//! household's readings, with a 7 kW charger, on Intelligent Octopus Go: over 855 half hours of October 2026 the
//! estimate agreed with the meter for all but 9, each of which were either a little charging at a low power or a short
//! high load in the house. A home battery charging outside the cheap times would be taken for the car.

use std::collections::{BTreeMap, BTreeSet};

use time_tz::Tz;

use super::measured::Split;
use super::topology::CAR;
use super::{Electricity, ElectricityPoint, EvComparison};

use super::grid::{self, GridPoint, GridReading, READING_SECONDS};
use crate::components::category_filter::order_labels;
use crate::period::Period;

/// The label of import which the meter has no reading for yet, where its rate can not be estimated, see Tariff.
pub const UNMETERED_LABEL: &str = "Not yet metered";
/// The app's labels of the Electricity page, with their default colours, see crate::known_labels: import not yet
/// metered, and import by use only, where there is no rate, e.g. from a Tesla gateway's readings alone. The rates'
/// labels, e.g. EV Low Rate, are made from the supplier's rates, so their colours are its module's.
pub const LABELS: [(&str, &str); 4] = [
    (UNMETERED_LABEL, "#8a8a99"),
    (HOUSE, "#46ee43"),
    (EV, "#271cff"),
    (BATTERY, "#9b51e0"),
];

/// The start of the labels of the house's, the car's and the batteries' import, e.g. EV Low Rate, see
/// topology::label_prefix.
const HOUSE: &str = "Home";
const EV: &str = "EV";
const BATTERY: &str = "Battery";

const HALF_HOUR: i64 = 1800;
/// A half hour has all of its Home Mini readings if none of them, nor its start or end, are further apart than this.
/// The meter reads about every 10.3 seconds, so about one 10 second slot in 35 has no reading.
const MAX_READING_GAP: i64 = 60;

/// A half hour with at least this much of the car's measured import, in kWh, had the car charging, for its estimated
/// rate, see Estimate.
const MEASURED_CHARGING: f64 = 0.1;

/// A half hour is estimated to have had the car charging if at least this many readings are at its charging power.
const CHARGING_READINGS: usize = 3;
/// The charging power is this much of the charger's power, so that the house's load on top is not needed.
const CHARGING_POWER_SHARE: f64 = 0.85;

/// How a supplier bills import, for estimating the rate of import its meter has not read yet, which its module gives, so
/// that a tariff's rules are known only to the module of the supplier whose tariff it is, see SiteData::estimate.
pub trait Tariff: std::fmt::Debug + Send + Sync {
    /// The rate the import of the half hour starting at half_hour, a unix timestamp, is estimated to be billed at, as the
    /// meter's readings name rates, e.g. Low Rate, see MeterReading::rates, given whether the car was charging in it,
    /// None if it can not be told.
    fn estimated_rate(&self, half_hour: i64, car_charging: bool) -> Option<String>;

    /// Whether the half hour starting at half_hour is in the tariff's cheap times, whatever the energy is used for, None
    /// if it has none or they are not known. Outside them the supplier's split of the car's import may decide its rate,
    /// see EvComparison.
    fn cheap_time(&self, half_hour: i64) -> Option<bool>;
}

/// What is known about the tariff and the car's charger, for estimating the rate of import which the meter has no
/// reading for yet, and placing the car's share of the import, see the module comment.
#[derive(Clone, Debug, Default)]
pub struct Estimate {
    /// How the supplier bills, None if it is not known.
    pub tariff: Option<std::sync::Arc<dyn Tariff>>,
    /// The charger's power, in W, None if there is no charger or it is not known.
    pub charger_watts: Option<f64>,
}

/// Where in a half hour the car's share of the import is placed, see split_half_hour.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Placement {
    /// It was charging in the half hours before and after too.
    Throughout,
    /// It was charging in the half hour before, so it carried on from the start.
    Start,
    /// It was charging in the half hour after, so it started before the end.
    End,
    /// Where the import is highest, as it was charging in neither the half hour before nor the one after. This assumes
    /// the car is the largest load, which it may not be, e.g. with a home battery charging.
    Highest,
}

/// A meter's reading, e.g. an Octopus half hour or a GloBird 5 minutes, with its start and end as unix timestamps: the
/// units of each label, in kWh, and what the supplier says they cost, in the minor units of SiteData::currency.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeterReading {
    pub start: i64,
    pub end: i64,
    pub units: BTreeMap<String, f64>,
    /// The rate each label is billed at, where that is not the label itself, e.g. Low Rate for Octopus's EV Low Rate and
    /// Home Low Rate, which are the supplier's split of that rate's kWh by use, which a measured split replaces.
    pub rates: BTreeMap<String, String>,
    /// The use the supplier billed a label as, where it says, a topology category, e.g. Car for Octopus's EV Low Rate.
    /// Labels without one are billed as the rest of the import, e.g. the house's and the batteries'.
    pub uses: BTreeMap<String, String>,
    /// The cost of each label, negative for a credit, e.g. for export, empty if it is not known.
    pub costs: BTreeMap<String, f64>,
    /// The reading's share of the standing charge.
    pub standing_charge: f64,
}

impl MeterReading {
    fn total(&self) -> f64 {
        self.units.values().sum()
    }

    /// The rate a label is billed at, see rates.
    fn rate<'a>(&'a self, label: &'a str) -> &'a str {
        self.rates.get(label).map_or(label, String::as_str)
    }

    /// The cost of its labels, e.g. the credit for the export.
    fn total_cost(&self) -> f64 {
        self.costs.values().sum()
    }

    /// The units of the labels billed as a use, a topology category, e.g. Car, see uses.
    fn units_of(&self, category: &str) -> f64 {
        self.units
            .iter()
            .filter(|(label, _)| self.uses.get(*label).is_some_and(|known| known == category))
            .map(|(_, units)| units)
            .sum()
    }

    /// The use and rate a label is billed as, see Billed.
    fn billed(&self, label: &str) -> Billed {
        (
            self.uses.get(label).cloned().unwrap_or_default(),
            self.rate(label).to_string(),
        )
    }
}

/// A group of a meter's kWh which are billed alike: the use the supplier billed them as, see MeterReading::uses, "" for
/// the rest, and their rate.
type Billed = (String, String);

/// Where a period's electricity is from, as its tooltip and the table show it, in this order: the grid meter, by its
/// name, e.g. Home Mini, what measured its use, e.g. Tesla, see Measurements::name, and the meters.
const GRID_SOURCE: u8 = 0;
const MEASURED_SOURCE: u8 = 1;
const METER_SOURCE: u8 = 2;

/// A source of a period's electricity and its place in the order, see GRID_SOURCE.
fn source(place: u8, grid_name: &str) -> (u8, String) {
    (
        place,
        String::from(if place == GRID_SOURCE { grid_name } else { "meter" }),
    )
}

/// A period's electricity while it is worked out, with the import by label.
#[derive(Default)]
struct PeriodTotals {
    start: i64,
    end: i64,
    import: BTreeMap<String, f64>,
    /// The use each label of the import is of, a topology category, e.g. Car, where it is known, see Electricity::uses.
    uses: BTreeMap<String, String>,
    export: f64,
    /// The cost of each label of the import, of the export and the standing charge, see MeterReading.
    costs: BTreeMap<String, f64>,
    export_cost: f64,
    standing_charge: f64,
    /// The Home Mini's average power, in W, for periods shorter than a half hour.
    demand: Option<f64>,
    /// Where it is from, each with its place in the order they are shown in, see source.
    sources: BTreeSet<(u8, String)>,
    ev_comparison: Option<EvComparison>,
}

impl PeriodTotals {
    fn add(&mut self, other: PeriodTotals) {
        self.demand = None;
        for (label, units) in other.import {
            *self.import.entry(label).or_default() += units;
        }
        self.uses.extend(other.uses);
        self.export += other.export;
        for (label, cost) in other.costs {
            *self.costs.entry(label).or_default() += cost;
        }
        self.export_cost += other.export_cost;
        self.standing_charge += other.standing_charge;
        self.sources.extend(other.sources);
        self.ev_comparison = match (self.ev_comparison, other.ev_comparison) {
            (Some(one), Some(other)) => Some(one + other),
            (one, other) => one.or(other),
        };
    }
}

/// The start of each half hour from start up to end, as unix timestamps, which the Home Mini has all the readings for,
/// see MAX_READING_GAP. readings are in time order. The half hour of the latest reading counts up to that reading, so
/// that the half hour in progress is shown.
fn complete_half_hours(readings: &[GridReading], start: i64, end: i64) -> Vec<i64> {
    let Some(latest) = readings.last().map(|reading| reading.read_at + READING_SECONDS) else {
        return Vec::new();
    };
    let mut complete = Vec::new();
    let mut half_hour = start - start.rem_euclid(HALF_HOUR);
    while half_hour < end.min(latest) {
        let half_hour_end = (half_hour + HALF_HOUR).min(latest);
        let first = readings.partition_point(|reading| reading.read_at < half_hour);
        let mut previous = half_hour;
        let mut gap_ok = true;
        let mut any = false;
        for reading in readings[first..]
            .iter()
            .take_while(|reading| reading.read_at < half_hour_end)
        {
            gap_ok &= reading.read_at - previous <= MAX_READING_GAP;
            previous = reading.read_at;
            any = true;
        }
        if any && gap_ok && half_hour_end - previous <= MAX_READING_GAP {
            complete.push(half_hour);
        }
        half_hour += HALF_HOUR;
    }
    complete
}

/// The import of each of a half hour's points, kwh, which are seconds long, split into the labels of the import meter's
/// reading for it. The import is the Home Mini's, which is very close to the meter's, so the meter's proportions are
/// used rather than its units. The car's share, the labels billed as the car's, see MeterReading::uses, is placed as a
/// block at the charger's power, if it is known, at the placement, and is otherwise shared in proportion to the points'
/// import. The rest of each point's import is the house's, in the proportions of the other labels.
fn split_half_hour(
    kwh: &[f64],
    seconds: &[i64],
    reading: &MeterReading,
    charger_watts: Option<f64>,
    placement: Placement,
) -> Vec<BTreeMap<String, f64>> {
    let units = &reading.units;
    let is_car = |label: &str| reading.uses.get(label).is_some_and(|category| category == CAR);
    let meter_total: f64 = units.values().sum();
    let ev_units = reading.units_of(CAR);
    let home_units = meter_total - ev_units;
    let total: f64 = kwh.iter().sum();
    let ev_total = total * ev_units / meter_total;

    // The most of each point which can be the car's.
    let room: Vec<f64> = match charger_watts {
        Some(watts) => kwh
            .iter()
            .zip(seconds)
            .map(|(kwh, seconds)| kwh.min(watts * *seconds as f64 / 3_600_000.0))
            .collect(),
        None => kwh.to_vec(),
    };
    let room_total: f64 = room.iter().sum();
    let mut ev = vec![0.0; kwh.len()];
    let fill = |ev: &mut Vec<f64>, order: &mut dyn Iterator<Item = usize>| {
        let mut left = ev_total;
        for index in order {
            ev[index] = room[index].min(left);
            left -= ev[index];
        }
    };
    match (charger_watts, placement) {
        (None, _) | (_, Placement::Throughout) => {
            if room_total > 0.0 {
                let share = (ev_total / room_total).min(1.0);
                for (ev, room) in ev.iter_mut().zip(&room) {
                    *ev = room * share;
                }
            }
        }
        (_, Placement::Start) => fill(&mut ev, &mut (0..kwh.len())),
        (_, Placement::End) => fill(&mut ev, &mut (0..kwh.len()).rev()),
        (_, Placement::Highest) => {
            // The run of points long enough for the car's share with the most import, then on from there.
            let average_room = room_total / room.len().max(1) as f64;
            let length = if average_room > 0.0 {
                ((ev_total / average_room).ceil() as usize).clamp(1, kwh.len().max(1))
            } else {
                1
            };
            let first = (0..=kwh.len().saturating_sub(length))
                .max_by(|a, b| {
                    kwh[*a..*a + length]
                        .iter()
                        .sum::<f64>()
                        .total_cmp(&kwh[*b..*b + length].iter().sum::<f64>())
                })
                .unwrap_or_default();
            fill(&mut ev, &mut (first..kwh.len()).chain(0..first));
        }
    }
    // Anything which did not fit, e.g. as the charger's power is a little low, is shared by the points with room.
    let left_over = ev_total - ev.iter().sum::<f64>();
    let space: f64 = kwh.iter().zip(&ev).map(|(kwh, ev)| kwh - ev).sum();
    if left_over > 1e-12 && space > 0.0 {
        for (ev, kwh) in ev.iter_mut().zip(kwh) {
            *ev += left_over * (kwh - *ev) / space;
        }
    }

    kwh.iter()
        .zip(ev)
        .map(|(kwh, ev)| {
            let home = kwh - ev;
            units
                .iter()
                .map(|(label, units)| {
                    let share = match (is_car(label), ev_units > 0.0, home_units > 0.0) {
                        (true, true, _) => ev * units / ev_units,
                        (false, _, true) => home * units / home_units,
                        (true, false, _) | (false, _, false) => 0.0,
                    };
                    (label.clone(), share)
                })
                // Import in a point which the meter gives none of to the house is given to the car.
                .chain((home_units == 0.0 && home > 0.0).then(|| {
                    let label = units.keys().find(|label| is_car(label)).cloned().unwrap_or_default();
                    (label, home)
                }))
                .fold(BTreeMap::new(), |mut labels, (label, kwh)| {
                    *labels.entry(label).or_insert(0.0) += kwh;
                    labels
                })
        })
        .collect()
}

/// The points with each half hour's import and export shared out between its points in proportion to the Home Mini's
/// power readings, rather than the changes in its registers between readings. The registers' totals are exact, but the
/// energy between two readings varies by several percent with the time between them, which is about 10.3 seconds
/// rather than the 10 their times say, and as the registers count whole Wh, which over 10 seconds looks like the power
/// changing when it is steady. A half hour with a point without a power reading is left as it is.
fn shaped_by_power(mut points: Vec<GridPoint>) -> Vec<GridPoint> {
    // Each point's energy in Wh at its power reading, import from positive power and export from negative.
    let at_power = |point: &GridPoint, sign: f64| {
        (sign * point.demand.unwrap_or_default()).max(0.0) * (point.end - point.start) as f64 / 3600.0
    };
    let half_hour = |point: &GridPoint| point.start - point.start.rem_euclid(HALF_HOUR);
    for group in points.chunk_by_mut(|a, b| half_hour(a) == half_hour(b)) {
        if group.iter().any(|point| point.demand.is_none()) {
            continue;
        }
        let (import, export): (f64, f64) = group.iter().fold((0.0, 0.0), |(import, export), point| {
            (import + point.energy, export + point.export)
        });
        let (import_power, export_power): (f64, f64) = group.iter().fold((0.0, 0.0), |(import, export), point| {
            (import + at_power(point, 1.0), export + at_power(point, -1.0))
        });
        for point in group.iter_mut() {
            if import_power > 0.0 {
                point.energy = import * at_power(point, 1.0) / import_power;
            }
            if export_power > 0.0 {
                point.export = export * at_power(point, -1.0) / export_power;
            }
        }
    }
    points
}

/// The labels of a point measured to have been used for each category, e.g. the car and the batteries, see
/// power::topology, in the rates, with the share at each, and with suffix, e.g. (estimated), each as PREFIX RATE SUFFIX,
/// e.g. EV Low Rate, see topology::label_prefix, or PREFIX SUFFIX for a rate which is not known, "". The use of each
/// label is added to label_uses.
fn measured_labels(
    uses: &BTreeMap<String, f64>,
    rates: &[(String, f64)],
    suffix: &str,
    label_uses: &mut BTreeMap<String, String>,
) -> BTreeMap<String, f64> {
    let mut labels = BTreeMap::new();
    for (rate, share) in rates {
        for (category, kwh) in uses {
            if *kwh > 0.0 {
                let kind = super::topology::label_prefix(category);
                let label = if rate.is_empty() {
                    format!("{kind}{suffix}")
                } else {
                    format!("{kind} {rate}{suffix}")
                };
                label_uses.insert(label.clone(), category.clone());
                *labels.entry(label).or_default() += kwh * share;
            }
        }
    }
    labels
}

/// What each use measured in a half hour, measured, by point, was billed as: the share of its kWh in each of the
/// meter's Billed groups. The kWh the supplier billed as a use go to that use, as far as it was measured to use them,
/// and the rest of the meter's kWh to the uses still short, in proportion to what they are short, so that the kWh in
/// each group over the half hour are the meter's, see the module comment.
fn billed_uses(reading: &MeterReading, measured: &[BTreeMap<String, f64>]) -> BTreeMap<String, Vec<(Billed, f64)>> {
    let mut wanted: BTreeMap<String, f64> = BTreeMap::new();
    for uses in measured {
        for (category, kwh) in uses.iter().filter(|(_, kwh)| **kwh > 0.0) {
            *wanted.entry(category.clone()).or_default() += kwh;
        }
    }
    let measured_total: f64 = wanted.values().sum();
    if measured_total <= 0.0 {
        return BTreeMap::new();
    }
    // In the meter's kWh, as the grid meter's differ a little.
    let scale = reading.total() / measured_total;
    wanted.values_mut().for_each(|kwh| *kwh *= scale);
    let mut pool: BTreeMap<Billed, f64> = BTreeMap::new();
    for (label, units) in reading.units.iter().filter(|(_, units)| **units > 0.0) {
        *pool.entry(reading.billed(label)).or_default() += units;
    }
    let mut given: BTreeMap<String, BTreeMap<Billed, f64>> = BTreeMap::new();
    for (category, want) in wanted.iter_mut() {
        let own: f64 = pool
            .iter()
            .filter(|((billed_use, _), _)| billed_use == category)
            .map(|(_, kwh)| kwh)
            .sum();
        if own <= 0.0 {
            continue;
        }
        let share = (*want / own).min(1.0);
        for (billed, kwh) in pool.iter_mut().filter(|((billed_use, _), _)| billed_use == category) {
            *given
                .entry(category.clone())
                .or_default()
                .entry(billed.clone())
                .or_default() += *kwh * share;
            *kwh *= 1.0 - share;
        }
        *want -= own * share;
    }
    let short: f64 = wanted.values().sum();
    if short > 0.0 {
        for (category, want) in &wanted {
            for (billed, kwh) in &pool {
                *given
                    .entry(category.clone())
                    .or_default()
                    .entry(billed.clone())
                    .or_default() += kwh * want / short;
            }
        }
    }
    given
        .into_iter()
        .map(|(category, billed)| {
            let total: f64 = billed.values().sum();
            let shares = billed
                .into_iter()
                .filter(|(_, kwh)| *kwh > 0.0)
                .map(|(billed, kwh)| (billed, kwh / total))
                .collect();
            (category, shares)
        })
        .collect()
}

/// The labels of each of a half hour's points, by the use measured, measured, at the rates it was billed at, see
/// billed_uses, e.g. EV Standard Rate, and the cost of each, at the meter's price of what it was billed as, made to add
/// up to the meter's cost over the half hour. The use of each label is added to label_uses.
fn billed_labels(
    reading: &MeterReading,
    measured: &[BTreeMap<String, f64>],
    label_uses: &mut BTreeMap<String, String>,
) -> (Vec<BTreeMap<String, f64>>, Vec<BTreeMap<String, f64>>) {
    let billed_uses = billed_uses(reading, measured);
    let mut billed_costs: BTreeMap<Billed, (f64, f64)> = BTreeMap::new();
    for (label, units) in &reading.units {
        let entry = billed_costs.entry(reading.billed(label)).or_default();
        entry.0 += units;
        entry.1 += reading.costs.get(label).copied().unwrap_or_default();
    }
    let price = |billed: &Billed| {
        billed_costs
            .get(billed)
            .filter(|(units, _)| *units > 0.0)
            .map_or(0.0, |(units, cost)| cost / units)
    };
    let mut labels = Vec::new();
    let mut costs = Vec::new();
    for uses in measured {
        let mut point_labels: BTreeMap<String, f64> = BTreeMap::new();
        let mut point_costs: BTreeMap<String, f64> = BTreeMap::new();
        for (category, kwh) in uses.iter().filter(|(_, kwh)| **kwh > 0.0) {
            for (billed, share) in billed_uses.get(category).into_iter().flatten() {
                let label = format!("{} {}", super::topology::label_prefix(category), billed.1);
                label_uses.insert(label.clone(), category.clone());
                *point_labels.entry(label.clone()).or_default() += kwh * share;
                *point_costs.entry(label).or_default() += kwh * share * price(billed);
            }
        }
        labels.push(point_labels);
        costs.push(point_costs);
    }
    let total: f64 = costs.iter().flat_map(|point| point.values()).sum();
    if total != 0.0 {
        let scale = reading.total_cost() / total;
        costs
            .iter_mut()
            .flat_map(|point| point.values_mut())
            .for_each(|cost| *cost *= scale);
    }
    (labels, costs)
}

/// The cost of each label of each of a half hour's points, labels, whose rates rate_of gives: the cost at each rate of
/// the meter's reading shared out among the labels at that rate, in proportion to their kWh, so that the cost at each
/// rate over the half hour is the supplier's, whoever is said to have used it.
fn shared_costs(
    reading: &MeterReading,
    labels: &[BTreeMap<String, f64>],
    rate_of: impl Fn(&str) -> String,
) -> Vec<BTreeMap<String, f64>> {
    let mut rate_costs: BTreeMap<&str, f64> = BTreeMap::new();
    for (label, cost) in &reading.costs {
        *rate_costs.entry(reading.rate(label)).or_default() += cost;
    }
    let mut rate_kwh: BTreeMap<String, f64> = BTreeMap::new();
    for point in labels {
        for (label, kwh) in point {
            *rate_kwh.entry(rate_of(label)).or_default() += kwh;
        }
    }
    labels
        .iter()
        .map(|point| {
            point
                .iter()
                .filter_map(|(label, kwh)| {
                    let rate = rate_of(label);
                    let cost = rate_costs.get(rate.as_str())?;
                    let total = rate_kwh.get(&rate).copied().unwrap_or_default();
                    (total > 0.0).then(|| (label.clone(), cost * kwh / total))
                })
                .collect()
        })
        .collect()
}

/// The start of each half hour, as unix timestamps, in which at least CHARGING_READINGS readings are at or above the
/// charging power, in W.
fn charging_half_hours(readings: &[GridReading], watts: f64) -> std::collections::BTreeSet<i64> {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    for reading in readings
        .iter()
        .filter(|reading| reading.demand.is_some_and(|demand| demand >= watts))
    {
        *counts
            .entry(reading.read_at - reading.read_at.rem_euclid(HALF_HOUR))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count >= CHARGING_READINGS)
        .map(|(half_hour, _)| half_hour)
        .collect()
}

/// The electricity from start to end, as unix timestamps, in periods of period, from the import and export meters'
/// half hours, the Home Mini's readings and the Tesla gateway's registers, see the module comment. Periods shorter than a half hour are only made from
/// the Home Mini's readings, half hours which only the meters have are shown as a half hour.
pub fn combine(
    import: &[MeterReading],
    export: &[MeterReading],
    readings: &[GridReading],
    grid_name: &str,
    measurements: Option<&dyn Split>,
    start: i64,
    end: i64,
    period: Period,
    estimate: &Estimate,
    tz: &'static Tz,
) -> Electricity {
    let import_meter: BTreeMap<i64, &MeterReading> =
        import.iter().map(|half_hour| (half_hour.start, half_hour)).collect();
    let export_meter: BTreeMap<i64, &MeterReading> =
        export.iter().map(|half_hour| (half_hour.start, half_hour)).collect();
    let complete: std::collections::BTreeSet<i64> = complete_half_hours(readings, start, end).into_iter().collect();
    let charging = estimate
        .charger_watts
        .map(|watts| charging_half_hours(readings, watts * CHARGING_POWER_SHARE))
        .unwrap_or_default();
    // Whether the supplier billed the car for any of the half hour starting at half_hour.
    let charged = |half_hour: i64| {
        import_meter
            .get(&half_hour)
            .is_some_and(|reading| reading.units_of(CAR) > 0.0)
    };

    // Whether the half hour starting at half_hour is in the tariff's cheap times, None if they are not known.
    let in_cheap_times = |half_hour: i64| estimate.tariff.as_ref()?.cheap_time(half_hour);
    // The rate the import in the half hour starting at half_hour, which the meter has no reading for, is estimated to be
    // billed at, None if it can not be estimated. measured_charging is whether the car was measured charging in it.
    let estimated_rate = |half_hour: i64, measured_charging: bool| {
        estimate
            .tariff
            .as_ref()?
            .estimated_rate(half_hour, measured_charging || charging.contains(&half_hour))
    };

    // The Home Mini's readings in the half hours it has them all for, in periods no longer than a half hour.
    let detail = match period.seconds() {
        Some(seconds) if seconds <= HALF_HOUR => period,
        _ => Period::HalfHour,
    };
    let points: Vec<GridPoint> = grid::aggregate(readings, start, end, detail, tz)
        .into_iter()
        .filter(|point| point.readings > 0 && complete.contains(&(point.start - point.start.rem_euclid(HALF_HOUR))))
        .collect();
    let points = shaped_by_power(points);

    // Each half hour's points, with their import split into labels, see split_half_hour.
    let mut parts: Vec<PeriodTotals> = Vec::new();
    for half_hour_points in
        points.chunk_by(|a, b| a.start - a.start.rem_euclid(HALF_HOUR) == b.start - b.start.rem_euclid(HALF_HOUR))
    {
        let half_hour = half_hour_points[0].start - half_hour_points[0].start.rem_euclid(HALF_HOUR);
        let kwh: Vec<f64> = half_hour_points.iter().map(|point| point.energy / 1000.0).collect();
        let seconds: Vec<i64> = half_hour_points.iter().map(|point| point.end - point.start).collect();
        let placement = match (charged(half_hour - HALF_HOUR), charged(half_hour + HALF_HOUR)) {
            (true, true) => Placement::Throughout,
            (true, false) => Placement::Start,
            (false, true) => Placement::End,
            (false, false) => Placement::Highest,
        };
        let bounds: Vec<(i64, i64)> = half_hour_points.iter().map(|point| (point.start, point.end)).collect();
        let reading = import_meter.get(&half_hour).filter(|reading| reading.total() > 0.0);
        let demand: Vec<Option<f64>> = half_hour_points.iter().map(|point| point.demand).collect();
        let measured = measurements.and_then(|measurements| measurements.split(&kwh, &demand, &bounds));
        let measured_ev: f64 = measured
            .iter()
            .flatten()
            .filter_map(|uses| uses.get(super::topology::CAR))
            .sum();
        // The labels of each point, and their costs, which only the meter's reading has, not an estimate, and the use of
        // each label, where it is known.
        let mut label_uses: BTreeMap<String, String> = BTreeMap::new();
        let (labels, costs, octopus_ev): (Vec<BTreeMap<String, f64>>, Vec<BTreeMap<String, f64>>, Option<f64>) =
            match (reading, &measured) {
                (Some(reading), Some(measured)) => {
                    let (labels, costs) = billed_labels(reading, measured, &mut label_uses);
                    (labels, costs, Some(reading.units_of(CAR)))
                }
                (Some(reading), _) => {
                    label_uses = reading.uses.clone();
                    let labels = split_half_hour(&kwh, &seconds, reading, estimate.charger_watts, placement);
                    let costs = shared_costs(reading, &labels, |label| reading.rate(label).to_string());
                    (labels, costs, None)
                }
                (None, measured) => {
                    let rate = estimated_rate(half_hour, measured_ev >= MEASURED_CHARGING);
                    let labels: Vec<BTreeMap<String, f64>> = match (rate, measured) {
                        (Some(rate), Some(measured)) => measured
                            .iter()
                            .map(|uses| measured_labels(uses, &[(rate.clone(), 1.0)], " (estimated)", &mut label_uses))
                            .collect(),
                        // Neither billed nor estimated, e.g. a site with no supplier's readings, so only by use.
                        (None, Some(measured)) => measured
                            .iter()
                            .map(|uses| measured_labels(uses, &[(String::new(), 1.0)], "", &mut label_uses))
                            .collect(),
                        // Not split by use, so the whole import's, labelled as the house's, as a supplier's would be.
                        (rate, None) => {
                            let label = match rate {
                                Some(rate) => format!(
                                    "{} {rate} (estimated)",
                                    super::topology::label_prefix(super::topology::HOUSE)
                                ),
                                None => String::from(UNMETERED_LABEL),
                            };
                            kwh.iter().map(|kwh| BTreeMap::from([(label.clone(), *kwh)])).collect()
                        }
                    };
                    let costs = vec![BTreeMap::new(); labels.len()];
                    (labels, costs, None)
                }
            };
        let mut sources = BTreeSet::from([source(GRID_SOURCE, grid_name)]);
        if let (Some(measurements), Some(_)) = (measurements, &measured) {
            sources.insert((MEASURED_SOURCE, measurements.name()));
        }
        // The standing charge shared out by time and the export's credit by the export.
        let standing_charge = import_meter.get(&half_hour).map_or(0.0, |reading| {
            reading.standing_charge / (reading.end - reading.start).max(1) as f64
        });
        let exported: f64 = half_hour_points.iter().map(|point| point.export).sum();
        let export_cost = |point: &GridPoint| {
            export_meter
                .get(&half_hour)
                .map_or(0.0, |reading| match exported > 0.0 {
                    true => reading.total_cost() * point.export / exported,
                    false => {
                        reading.total_cost() * (point.end - point.start) as f64
                            / (reading.end - reading.start).max(1) as f64
                    }
                })
        };
        let outside = in_cheap_times(half_hour) == Some(false);
        parts.extend(
            half_hour_points
                .iter()
                .zip(labels)
                .zip(costs)
                .map(|((point, import), costs)| {
                    let ev_comparison = octopus_ev.map(|octopus| {
                        let share = (point.end - point.start) as f64 / HALF_HOUR as f64;
                        let (measured, octopus) = (measured_ev * share, octopus * share);
                        EvComparison {
                            measured,
                            octopus,
                            measured_outside: if outside { measured } else { 0.0 },
                            octopus_outside: if outside { octopus } else { 0.0 },
                        }
                    });
                    PeriodTotals {
                        start: point.start,
                        end: point.end,
                        import,
                        uses: label_uses.clone(),
                        export: point.export / 1000.0,
                        costs,
                        export_cost: export_cost(point),
                        standing_charge: standing_charge * (point.end - point.start) as f64,
                        demand: point.demand,
                        sources: sources.clone(),
                        ev_comparison,
                    }
                }),
        );
    }

    // The meters' readings of the half hours the grid meter has not all of, however long they are, e.g. GloBird's 5
    // minutes, from the start of start's half hour.
    let first = start - start.rem_euclid(HALF_HOUR);
    let mut meter_parts: BTreeMap<i64, PeriodTotals> = BTreeMap::new();
    for (reading, exported) in import
        .iter()
        .map(|reading| (reading, false))
        .chain(export.iter().map(|reading| (reading, true)))
    {
        if reading.start < first
            || reading.start >= end
            || complete.contains(&(reading.start - reading.start.rem_euclid(HALF_HOUR)))
        {
            continue;
        }
        let part = meter_parts.entry(reading.start).or_insert_with(|| PeriodTotals {
            start: reading.start,
            end: reading.end,
            sources: BTreeSet::from([source(METER_SOURCE, grid_name)]),
            ..PeriodTotals::default()
        });
        if exported {
            part.export += reading.total();
            part.export_cost += reading.total_cost();
        } else {
            part.import = reading.units.clone();
            part.uses = reading.uses.clone();
            part.costs = reading.costs.clone();
            part.standing_charge = reading.standing_charge;
        }
    }
    parts.extend(meter_parts.into_values());

    // Periods of a half hour or longer add the parts in each together.
    let periods: Vec<PeriodTotals> = if detail == period && period != Period::HalfHour {
        // The grid meter's points are of the period already, but meters' readings shorter than it, e.g. GloBird's 5
        // minutes in 10 minute periods, are added together into it.
        let seconds = period.seconds().unwrap_or(i64::MAX);
        let mut grouped: BTreeMap<i64, PeriodTotals> = BTreeMap::new();
        let mut kept = Vec::new();
        for part in parts {
            let meter_only = part.sources.iter().all(|(place, _)| *place == METER_SOURCE);
            if meter_only && part.end - part.start < seconds {
                let (period_start, period_end) = crate::period::period_bounds(part.start, period, tz);
                grouped
                    .entry(period_start)
                    .or_insert_with(|| PeriodTotals {
                        start: period_start,
                        end: period_end,
                        ..PeriodTotals::default()
                    })
                    .add(part);
            } else {
                kept.push(part);
            }
        }
        kept.extend(grouped.into_values());
        kept.sort_by_key(|part| part.start);
        kept
    } else {
        let mut periods: BTreeMap<i64, PeriodTotals> = BTreeMap::new();
        for part in parts {
            let (period_start, period_end) = crate::period::period_bounds(part.start, period, tz);
            periods
                .entry(period_start)
                .or_insert_with(|| PeriodTotals {
                    start: period_start,
                    end: period_end,
                    ..PeriodTotals::default()
                })
                .add(part);
        }
        periods.into_values().collect()
    };

    let labels = order_labels(
        periods
            .iter()
            .flat_map(|period| period.import.keys().map(String::as_str)),
    );
    let label_uses: BTreeMap<&String, &String> = periods.iter().flat_map(|period| &period.uses).collect();
    let uses = labels
        .iter()
        .map(|label| {
            label_uses
                .get(label)
                .map(|category| category.to_string())
                .unwrap_or_default()
        })
        .collect();
    let points = periods
        .into_iter()
        .map(|period| ElectricityPoint {
            start: period.start,
            end: period.end,
            import: labels
                .iter()
                .map(|label| period.import.get(label).copied().unwrap_or_default())
                .collect(),
            export: period.export,
            costs: labels
                .iter()
                .map(|label| period.costs.get(label).copied().unwrap_or_default())
                .collect(),
            export_cost: period.export_cost,
            standing_charge: period.standing_charge,
            demand: period.demand,
            // Once each, as the grid meter and what measured the use may be the same, e.g. Tesla.
            sources: period.sources.into_iter().fold(Vec::new(), |mut names, (_, name)| {
                if !names.contains(&name) {
                    names.push(name);
                }
                names
            }),
            ev_comparison: period.ev_comparison,
        })
        .collect();
    Electricity {
        labels,
        uses,
        points,
        currency: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::power::EXPORT_LABEL;
    use std::sync::Arc;

    use super::{
        Estimate, MeterReading, Placement, Tariff, UNMETERED_LABEL, combine, complete_half_hours, shaped_by_power,
        split_half_hour,
    };
    use crate::period::Period;
    use crate::power::grid::GridPoint;
    use crate::power::grid::GridReading;
    use crate::power::measured::RegisterMeasurements;
    use crate::power::topology::TopologySplit;

    /// The gateway's registers, see power::measured::tests::registers, as the Tesla module gives them, placed by the
    /// first household's topology, with the car's charger beside the gateway.
    fn tesla(start: i64, end: i64, site_watts: f64, battery_watts: f64) -> TopologySplit {
        let registers = crate::power::measured::tests::registers(start, end, site_watts, battery_watts);
        TopologySplit {
            topology: crate::power::topology::tests::beside_the_gateway(),
            measurements: vec![Arc::new(RegisterMeasurements {
                name: String::from("Tesla"),
                registers,
            })],
        }
    }

    const LONDON: &time_tz::Tz = time_tz::timezones::db::europe::LONDON;
    /// Midnight UTC, in GMT, so also local midnight.
    const DAY: i64 = 1_793_232_000;

    /// A reading every 10 seconds from start up to end, importing 10 Wh and exporting 1 Wh each, with the registers
    /// starting at the given values.
    fn readings(start: i64, end: i64) -> Vec<GridReading> {
        (start..end)
            .step_by(10)
            .enumerate()
            .map(|(index, read_at)| GridReading {
                read_at,
                demand: Some(3600.0),
                consumption_delta: Some(10.0),
                consumption: Some(1000.0 + 10.0 * index as f64),
                export: Some(500.0 + index as f64),
            })
            .collect()
    }

    /// A meter's half hour with labels as Octopus's are, each billed at its rate, e.g. Low Rate for EV Low Rate, and the
    /// EV ones as the car's.
    fn half_hour(start: i64, units: &[(&str, f64)]) -> MeterReading {
        MeterReading {
            start,
            end: start + 1800,
            units: units.iter().map(|(label, units)| (label.to_string(), *units)).collect(),
            rates: units
                .iter()
                .filter_map(|(label, _)| {
                    Some((
                        label.to_string(),
                        label
                            .strip_prefix("Home ")
                            .or_else(|| label.strip_prefix("EV "))?
                            .to_string(),
                    ))
                })
                .collect(),
            uses: units
                .iter()
                .filter(|(label, _)| label.starts_with("EV "))
                .map(|(label, _)| (label.to_string(), String::from(crate::power::topology::CAR)))
                .collect(),
            ..MeterReading::default()
        }
    }

    #[test]
    fn a_half_hour_has_all_its_readings_if_none_are_more_than_a_minute_apart() {
        let mut all = readings(DAY - 10, DAY + 7200);
        assert_eq!(
            complete_half_hours(&all, DAY, DAY + 7200),
            [DAY, DAY + 1800, DAY + 3600, DAY + 5400]
        );
        // Two minutes missing from the second half hour, and the last half hour only up to its latest reading.
        all.retain(|reading| !(DAY + 2000..DAY + 2120).contains(&reading.read_at) && reading.read_at < DAY + 6000);
        assert_eq!(
            complete_half_hours(&all, DAY, DAY + 7200),
            [DAY, DAY + 3600, DAY + 5400]
        );
        assert!(complete_half_hours(&[], DAY, DAY + 7200).is_empty());
    }

    #[test]
    fn the_home_mini_is_used_where_it_is_complete_split_by_the_meters_labels_and_the_meters_elsewhere() {
        // The Home Mini has the second half hour, the meters the first two, and nothing has the third.
        let all = readings(DAY + 1790, DAY + 3600);
        let import = [
            half_hour(DAY, &[("Home Standard Rate", 2.0)]),
            half_hour(DAY + 1800, &[("Home Low Rate", 1.0), ("EV Low Rate", 3.0)]),
        ];
        let export = [
            half_hour(DAY, &[("GENERATION", 0.5)]),
            half_hour(DAY + 1800, &[("GENERATION", 0.25)]),
        ];
        let electricity = combine(
            &import,
            &export,
            &all,
            "Home Mini",
            None,
            DAY,
            DAY + 5400,
            Period::HalfHour,
            &Estimate::default(),
            LONDON,
        );
        assert_eq!(
            electricity.labels,
            ["Home Low Rate", "Home Standard Rate", "EV Low Rate"]
        );
        let points = &electricity.points;
        assert_eq!(points.len(), 2);
        assert_eq!(
            (points[0].start, points[0].import.clone(), points[0].export),
            (DAY, vec![0.0, 2.0, 0.0], 0.5)
        );
        assert_eq!(points[0].sources, ["meter"]);
        // 180 readings of 10 Wh, split 1 to 3 as the meter's reading is, and 180 of 1 Wh exported.
        assert_eq!(points[1].start, DAY + 1800);
        assert!((points[1].import[0] - 0.45).abs() < 1e-9 && (points[1].import[2] - 1.35).abs() < 1e-9);
        assert!((points[1].export - 0.18).abs() < 1e-9);
        assert_eq!(points[1].sources, ["Home Mini"]);
    }

    #[test]
    fn import_the_meter_has_not_read_yet_is_unmetered_and_shorter_periods_are_from_the_home_mini() {
        let all = readings(DAY - 10, DAY + 1800);
        let electricity = combine(
            &[],
            &[],
            &all,
            "Home Mini",
            None,
            DAY,
            DAY + 1800,
            Period::TenMinutes,
            &Estimate::default(),
            LONDON,
        );
        assert_eq!(electricity.labels, [UNMETERED_LABEL]);
        let starts: Vec<i64> = electricity.points.iter().map(|point| point.start).collect();
        assert_eq!(starts, [DAY, DAY + 600, DAY + 1200]);
        assert!(
            electricity
                .points
                .iter()
                .all(|point| (point.import[0] - 0.6).abs() < 1e-9)
        );
        assert_ne!(EXPORT_LABEL, UNMETERED_LABEL);
    }

    #[test]
    fn a_day_adds_the_home_mini_and_the_meters_half_hours_together() {
        let all = readings(DAY + 1790, DAY + 3600);
        let import = [
            half_hour(DAY, &[("Home Low Rate", 2.0)]),
            half_hour(DAY + 1800, &[("Home Low Rate", 9.0)]),
        ];
        let electricity = combine(
            &import,
            &[],
            &all,
            "Home Mini",
            None,
            DAY,
            DAY + 86400,
            Period::Day,
            &Estimate::default(),
            LONDON,
        );
        assert_eq!(electricity.points.len(), 1);
        let day = &electricity.points[0];
        assert_eq!((day.start, day.end), (DAY, DAY + 86400));
        // The meter's 2 kWh and the Home Mini's 1.8 kWh, rather than the meter's 9 kWh for the second half hour.
        assert!((day.import[0] - 3.8).abs() < 1e-9);
        assert_eq!(day.sources, ["Home Mini", "meter"]);
    }

    /// A tariff whose cheap times are 23:30 to 05:30 UTC, on which charging the car is at the low rate if
    /// cheap_charging, as on Intelligent Octopus.
    #[derive(Debug)]
    struct NightTariff {
        cheap_charging: bool,
    }

    impl Tariff for NightTariff {
        fn estimated_rate(&self, half_hour: i64, car_charging: bool) -> Option<String> {
            let low = self.cheap_time(half_hour)? || (self.cheap_charging && car_charging);
            Some(String::from(if low { "Low Rate" } else { "Standard Rate" }))
        }

        fn cheap_time(&self, half_hour: i64) -> Option<bool> {
            let minute = half_hour.rem_euclid(86400) / 60;
            Some(minute >= 23 * 60 + 30 || minute < 5 * 60 + 30)
        }
    }

    fn night(cheap_charging: bool) -> Estimate {
        Estimate {
            tariff: Some(Arc::new(NightTariff { cheap_charging })),
            charger_watts: Some(7000.0),
        }
    }

    #[test]
    fn unmetered_import_is_low_rate_in_the_cheap_times_or_while_charging_and_otherwise_standard() {
        let estimate = night(true);
        // From 05:00 to 06:30, with the car charging at 7 kW from 06:00.
        let mut all = readings(DAY + 5 * 3600 - 10, DAY + 6 * 3600 + 1800);
        for reading in all.iter_mut().filter(|reading| reading.read_at >= DAY + 6 * 3600) {
            reading.demand = Some(7000.0);
        }
        let electricity = combine(
            &[],
            &[],
            &all,
            "Home Mini",
            None,
            DAY + 5 * 3600,
            DAY + 6 * 3600 + 1800,
            Period::HalfHour,
            &estimate,
            LONDON,
        );
        assert_eq!(
            electricity.labels,
            ["Home Low Rate (estimated)", "Home Standard Rate (estimated)"]
        );
        let rates: Vec<Vec<bool>> = electricity
            .points
            .iter()
            .map(|point| point.import.iter().map(|kwh| *kwh > 0.0).collect())
            .collect();
        assert_eq!(rates, [vec![true, false], vec![false, true], vec![true, false]]);
        // On a tariff which is not an Intelligent Octopus one, charging outside the cheap times is at the standard rate.
        let other = night(false);
        let electricity = combine(
            &[],
            &[],
            &all,
            "Home Mini",
            None,
            DAY + 5 * 3600,
            DAY + 6 * 3600 + 1800,
            Period::HalfHour,
            &other,
            LONDON,
        );
        let rates: Vec<Vec<bool>> = electricity
            .points
            .iter()
            .map(|point| point.import.iter().map(|kwh| *kwh > 0.0).collect())
            .collect();
        assert_eq!(rates, [vec![true, false], vec![false, true], vec![false, true]]);
    }

    #[test]
    fn the_cars_share_of_a_metered_half_hour_is_a_block_at_the_chargers_power_where_it_was_charging() {
        // Six 10 second points, the car's share 0.06 kWh, which is three points at 7.2 kW (0.02 kWh each).
        let units = half_hour(DAY, &[("EV Low Rate", 1.0), ("Home Low Rate", 4.0)]);
        let seconds = [10; 6];
        let ev = |kwh: &[f64], placement: Placement| -> Vec<f64> {
            split_half_hour(kwh, &seconds, &units, Some(7200.0), placement)
                .iter()
                .map(|labels| labels["EV Low Rate"])
                .collect()
        };
        let close = |actual: Vec<f64>, expected: [f64; 6]| {
            actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 1e-9)
        };
        let flat = [0.05; 6];
        assert!(close(ev(&flat, Placement::Start), [0.02, 0.02, 0.02, 0.0, 0.0, 0.0]));
        assert!(close(ev(&flat, Placement::End), [0.0, 0.0, 0.0, 0.02, 0.02, 0.02]));
        assert!(close(ev(&flat, Placement::Throughout), [0.01; 6]));
        // Where the import is highest, as the car adds its power to the house's: its 0.048 kWh share of 0.24 kWh from the
        // first of the three highest points.
        let step = [0.03, 0.03, 0.05, 0.05, 0.05, 0.03];
        assert!(close(ev(&step, Placement::Highest), [0.0, 0.0, 0.02, 0.02, 0.008, 0.0]));
        // Every point's labels add up to its import.
        let split = split_half_hour(&step, &seconds, &units, Some(7200.0), Placement::Highest);
        assert!(
            split
                .iter()
                .zip(step)
                .all(|(labels, kwh)| (labels.values().sum::<f64>() - kwh).abs() < 1e-9)
        );
    }

    #[test]
    fn a_half_hours_energy_is_shared_out_by_the_power_readings() {
        let point = |start: i64, energy: f64, demand: f64| GridPoint {
            start,
            end: start + 10,
            energy,
            export: 0.0,
            demand: Some(demand),
            readings: 1,
        };
        // Steady power, but the energy between readings jittering, and a total of 120 Wh.
        let shaped = shaped_by_power(vec![
            point(DAY, 39.0, 14000.0),
            point(DAY + 10, 43.0, 14000.0),
            point(DAY + 20, 38.0, 14000.0),
        ]);
        assert!(shaped.iter().all(|point| (point.energy - 40.0).abs() < 1e-9));
        // A half hour with a point without a power reading is left as it is.
        let unshaped = shaped_by_power(vec![
            point(DAY, 39.0, 14000.0),
            GridPoint {
                demand: None,
                ..point(DAY + 10, 43.0, 0.0)
            },
        ]);
        assert_eq!(
            unshaped.iter().map(|point| point.energy).collect::<Vec<_>>(),
            [39.0, 43.0]
        );
    }

    #[test]
    fn a_measured_half_hour_is_labelled_by_use_at_the_meters_rate_with_octopuss_split_to_compare() {
        // The Home Mini's 3.6 kW (1.8 kWh), the gateway's 1.8 kW, so the car 0.9 kWh, which Octopus put at 1.5 kWh.
        let all = readings(DAY - 10, DAY + 1810);
        let import = [half_hour(DAY, &[("Home Low Rate", 0.3), ("EV Low Rate", 1.5)])];
        let electricity = combine(
            &import,
            &[],
            &all,
            "Home Mini",
            Some(&tesla(DAY - 20, DAY + 1820, 1800.0, 0.0)),
            DAY,
            DAY + 1800,
            Period::HalfHour,
            &Estimate::default(),
            LONDON,
        );
        assert_eq!(electricity.labels, ["Home Low Rate", "EV Low Rate"]);
        let point = &electricity.points[0];
        assert!(
            (point.import[0] - 0.9).abs() < 1e-6 && (point.import[1] - 0.9).abs() < 1e-6,
            "{:?}",
            point.import
        );
        assert_eq!(point.sources, ["Home Mini", "Tesla"]);
        let comparison = point.ev_comparison.unwrap();
        assert!((comparison.measured - 0.9).abs() < 1e-6 && (comparison.octopus - 1.5).abs() < 1e-9);
        // Without the gateway's registers, the meter's split, and nothing to compare.
        let electricity = combine(
            &import,
            &[],
            &all,
            "Home Mini",
            None,
            DAY,
            DAY + 1800,
            Period::HalfHour,
            &Estimate::default(),
            LONDON,
        );
        assert!(electricity.points[0].sources == ["Home Mini"] && electricity.points[0].ev_comparison.is_none());
    }

    #[test]
    fn the_cost_at_each_rate_is_the_meters_shared_out_by_the_measured_use() {
        // As above, the car measured at 0.9 kWh of the 1.8, which Octopus put at 1.5, all at the low rate, 13.5p, and a
        // standing charge of 2p.
        let all = readings(DAY - 10, DAY + 1810);
        let mut reading = half_hour(DAY, &[("Home Low Rate", 0.3), ("EV Low Rate", 1.5)]);
        reading.costs = BTreeMap::from([
            (String::from("Home Low Rate"), 2.25),
            (String::from("EV Low Rate"), 11.25),
        ]);
        reading.standing_charge = 2.0;
        let mut export = half_hour(DAY, &[(EXPORT_LABEL, 0.5)]);
        export.costs = BTreeMap::from([(String::from(EXPORT_LABEL), -7.5)]);
        let electricity = combine(
            &[reading.clone()],
            &[export],
            &all,
            "Home Mini",
            Some(&tesla(DAY - 20, DAY + 1820, 1800.0, 0.0)),
            DAY,
            DAY + 1800,
            Period::HalfHour,
            &Estimate::default(),
            LONDON,
        );
        let point = &electricity.points[0];
        assert!(
            (point.costs[0] - 6.75).abs() < 1e-6 && (point.costs[1] - 6.75).abs() < 1e-6,
            "{:?}",
            point.costs
        );
        assert!(
            (point.standing_charge - 2.0).abs() < 1e-9 && (point.export_cost + 7.5).abs() < 1e-9,
            "{point:?}"
        );
        // In 10 minutes, the same over the half hour.
        let electricity = combine(
            &[reading],
            &[],
            &all,
            "Home Mini",
            Some(&tesla(DAY - 20, DAY + 1820, 1800.0, 0.0)),
            DAY,
            DAY + 1800,
            Period::TenMinutes,
            &Estimate::default(),
            LONDON,
        );
        let total: f64 = electricity.points.iter().flat_map(|point| point.costs.iter()).sum();
        let standing: f64 = electricity.points.iter().map(|point| point.standing_charge).sum();
        assert!(
            (total - 13.5).abs() < 1e-6 && (standing - 2.0).abs() < 1e-6,
            "{:?}",
            electricity.points
        );
    }

    #[test]
    fn what_the_supplier_billed_as_the_cars_goes_to_the_measured_car_first_and_the_rest_at_its_own_rate() {
        // As on 3 Nov 2026 04:30, Octopus billed what it took for the car at the peak rate, past its limit, and the house
        // at the low rate. Measured, the car took 0.9 kWh, the house 0.9 kWh.
        let all = readings(DAY - 10, DAY + 1810);
        let tesla = tesla(DAY - 20, DAY + 1820, 1800.0, 0.0);
        let combined = |reading: MeterReading| {
            let electricity = combine(
                &[reading],
                &[],
                &all,
                "Home Mini",
                Some(&tesla),
                DAY,
                DAY + 1800,
                Period::HalfHour,
                &Estimate::default(),
                LONDON,
            );
            let point = &electricity.points[0];
            let value = |values: &[f64], label: &str| {
                electricity
                    .labels
                    .iter()
                    .position(|known| known == label)
                    .map_or(0.0, |index| values[index])
            };
            ["EV Standard Rate", "EV Low Rate", "Home Standard Rate", "Home Low Rate"].map(|label| {
                (
                    (value(&point.import, label) * 1000.0).round() / 1000.0,
                    (value(&point.costs, label) * 100.0).round() / 100.0,
                )
            })
        };
        let costed = |units: &[(&str, f64)], prices: &[f64]| {
            let mut reading = half_hour(DAY, units);
            reading.costs = units
                .iter()
                .zip(prices)
                .map(|((label, units), price)| (label.to_string(), units * price))
                .collect();
            reading
        };
        // Octopus took 1.5 kWh for the car, so the 0.6 the car did not take is the house's, at the peak rate it was billed.
        assert_eq!(
            combined(costed(
                &[("EV Standard Rate", 1.5), ("Home Low Rate", 0.3)],
                &[36.294, 6.6]
            )),
            [(0.9, 32.66), (0.0, 0.0), (0.6, 21.78), (0.3, 1.98)]
        );
        // Octopus took 0.5 kWh for the car, so the car's other 0.4 is from what it billed as the house's, at the low rate.
        assert_eq!(
            combined(costed(
                &[("EV Standard Rate", 0.5), ("Home Low Rate", 1.3)],
                &[36.294, 6.6]
            )),
            [(0.5, 18.15), (0.4, 2.64), (0.0, 0.0), (0.9, 5.94)]
        );
    }

    #[test]
    fn the_comparison_is_as_much_each_minute_of_the_half_hour_and_counted_apart_outside_the_cheap_times() {
        let estimate = Estimate {
            charger_watts: None,
            ..night(false)
        };
        // As a_measured_half_hour_is_labelled_by_use_at_the_meters_rate_with_octopuss_split_to_compare, at midnight,
        // in the cheap times, and at noon, outside them.
        for (start, outside) in [(DAY, false), (DAY + 12 * 3600, true)] {
            let all = readings(start - 10, start + 1810);
            let import = [half_hour(start, &[("Home Low Rate", 0.3), ("EV Low Rate", 1.5)])];
            let electricity = combine(
                &import,
                &[],
                &all,
                "Home Mini",
                Some(&tesla(start - 20, start + 1820, 1800.0, 0.0)),
                start,
                start + 1800,
                Period::Minute,
                &estimate,
                LONDON,
            );
            assert_eq!(electricity.points.len(), 30);
            for point in &electricity.points {
                let comparison = point.ev_comparison.unwrap();
                assert!(
                    (comparison.measured - 0.03).abs() < 1e-6 && (comparison.octopus - 0.05).abs() < 1e-9,
                    "{comparison:?}"
                );
                assert_eq!(
                    comparison.octopus_outside,
                    if outside { comparison.octopus } else { 0.0 }
                );
                assert_eq!(
                    comparison.measured_outside,
                    if outside { comparison.measured } else { 0.0 }
                );
            }
        }
    }
}
