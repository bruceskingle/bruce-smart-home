//! The tariff's rates, worked out from the cached days, as GloBird's portal gives no rates: only each day's cost of
//! each category, USAGE, SOLAR and SUPPLY, rounded to the cent, and the meter's kWh by charge type, e.g. Peak Usage.
//! A day's USAGE cost is the sum of its import's charge types' kWh at their rates, and its SOLAR credit that of its
//! export's, so the rates are those which best fit the days, by least squares, and the daily charge is that of SUPPLY.
//!
//! Rates change, e.g. on GloBird's review each 1 July, or with a new plan, so the days are split into runs which each
//! have rates of their own, where one set of rates does not fit them all, see runs. A day whose charged kWh differs
//! from the meter's, as GloBird sometimes moves kWh from one day to the next, is left out of the fit, and each day is
//! checked against its run's rates, see DayCheck.

use std::collections::BTreeMap;

use time::Date;

use super::fetch::DaySummary;

/// GloBird's cost categories of the import's and the export's kWh, and of the daily charge.
pub const USAGE: &str = "USAGE";
pub const SOLAR: &str = "SOLAR";
pub const SUPPLY: &str = "SUPPLY";

/// How far a day's cost may be from its kWh at the rates, in AUD, and still fit: the cent it is rounded to, and more,
/// for the rates being worked out from days which are rounded too.
const TOLERANCE: f64 = 0.02;
/// How far a day's charged kWh may be from the meter's, as the readings are of 3 decimal places.
const KWH_TOLERANCE: f64 = 0.05;
/// The fewest kWh a charge type must have over a run for its rate to be worked out, as one little used, e.g. Peak
/// Usage for a household whose batteries run it then, has too little in the costs to tell its rate.
const MIN_KWH: f64 = 1.0;
/// The fewest days in a run beyond the rates it has to work out, so that a few odd days do not make a run of their own.
const MIN_EXTRA_DAYS: usize = 3;

/// The rate of a charge type, in AUD a kWh, negative for a credit, e.g. a feed-in tariff, None if it could not be worked
/// out, see MIN_KWH.
#[derive(Debug, Clone, PartialEq)]
pub struct Rate {
    pub charge_type: String,
    pub rate: Option<f64>,
}

/// The rates of a run of days.
#[derive(Debug, Clone, PartialEq)]
pub struct Tariff {
    pub first: Date,
    pub last: Date,
    /// The import's charge types' rates, e.g. Peak Usage's.
    pub usage: Vec<Rate>,
    /// The export's, e.g. Standard Solar Feed in's.
    pub feed_in: Vec<Rate>,
    /// The daily charge, in AUD, the most common of the run's days'.
    pub daily: Option<f64>,
    /// How many days the rates were worked out from.
    pub days: usize,
}

impl Tariff {
    /// The rate of a charge type of the import, if it is USAGE, or of the export.
    pub fn rate(&self, category: &str, charge_type: &str) -> Option<f64> {
        let rates = if category == USAGE { &self.usage } else { &self.feed_in };
        rates.iter().find(|rate| rate.charge_type == charge_type)?.rate
    }

    /// The cost of a category of a day at these rates: each charge type's kWh at its rate. None if a charge type with
    /// kWh has no rate.
    pub fn cost(&self, day: &DaySummary, category: &str) -> Option<f64> {
        let mut cost = 0.0;
        for (charge_type, kwh) in kwh(day, category) {
            if kwh.abs() > 0.0005 {
                cost += kwh * self.rate(category, charge_type)?;
            }
        }
        Some(cost)
    }
}

/// How a day's charges compare with its kWh at its run's rates, for a category, USAGE or SOLAR, which it does not fit.
#[derive(Debug, Clone, PartialEq)]
pub struct DayCheck {
    pub category: String,
    /// What GloBird charged, in AUD, and the kWh at the rates.
    pub charged: f64,
    pub expected: f64,
    /// The kWh GloBird charged for, and the meter's, if they differ.
    pub kwh: Option<(f64, f64)>,
}

/// The import's or the export's kWh of a day by charge type, for the category, USAGE or SOLAR.
fn kwh<'a>(day: &'a DaySummary, category: &str) -> &'a BTreeMap<String, f64> {
    if category == USAGE { &day.import } else { &day.export_by_type }
}

/// Whether the kWh GloBird charged a category of a day for are the meter's, as they are unless it has moved some
/// to another day.
fn kwh_agree(day: &DaySummary, category: &str) -> bool {
    match day.quantity.get(category) {
        Some(quantity) => (quantity - kwh(day, category).values().sum::<f64>()).abs() < KWH_TOLERANCE,
        None => true,
    }
}

/// Whether a day can be fitted, for a category: it has all its readings, actual ones, and its cost, of the kWh the
/// meter measured.
fn fittable(day: &DaySummary, category: &str) -> bool {
    day.readings == 288 && !day.estimated && !day.cost_read_missing && day.cost.contains_key(category) && kwh_agree(day, category)
}

/// The x which best solves a x = b, by least squares, None if the rows do not determine it.
fn least_squares(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = a.first()?.len();
    // The normal equations, aᵀa x = aᵀb, solved by Gaussian elimination with partial pivoting.
    let mut m: Vec<Vec<f64>> = (0..n).map(|i| {
        let mut row: Vec<f64> = (0..n).map(|j| a.iter().map(|r| r[i] * r[j]).sum()).collect();
        row.push(a.iter().zip(b).map(|(r, y)| r[i] * y).sum());
        row
    }).collect();
    for column in 0..n {
        let pivot = (column..n).max_by(|&x, &y| m[x][column].abs().total_cmp(&m[y][column].abs()))?;
        if m[pivot][column].abs() < 1e-9 {
            return None;
        }
        m.swap(column, pivot);
        for row in 0..n {
            if row != column {
                let factor = m[row][column] / m[column][column];
                for k in column..=n {
                    m[row][k] -= factor * m[column][k];
                }
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}

/// The rates of a category's charge types which best fit days, see the module comment.
fn fit(days: &[&DaySummary], category: &str) -> Vec<Rate> {
    let fitted: Vec<&DaySummary> = days.iter().copied().filter(|day| fittable(day, category)).collect();
    let mut totals: BTreeMap<&str, f64> = BTreeMap::new();
    for day in &fitted {
        for (charge_type, kwh) in kwh(day, category) {
            *totals.entry(charge_type.as_str()).or_default() += kwh;
        }
    }
    let known: Vec<&str> = totals.iter().filter(|(_, total)| total.abs() >= MIN_KWH).map(|(charge_type, _)| *charge_type).collect();
    let rates = if fitted.len() >= known.len() + MIN_EXTRA_DAYS {
        let a: Vec<Vec<f64>> = fitted.iter().map(|day| known.iter().map(|charge_type| kwh(day, category).get(*charge_type).copied().unwrap_or_default()).collect()).collect();
        let b: Vec<f64> = fitted.iter().map(|day| day.cost[category]).collect();
        least_squares(&a, &b)
    }
    else {
        None
    };
    totals.keys().map(|charge_type| Rate {
        charge_type: charge_type.to_string(),
        rate: rates.as_ref().and_then(|rates| known.iter().position(|known| known == charge_type).map(|index| rates[index])),
    }).collect()
}

/// The most common daily charge of days, None if none has one.
fn daily(days: &[&DaySummary]) -> Option<f64> {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    for day in days {
        if let Some(supply) = day.cost.get(SUPPLY) {
            *counts.entry((supply * 100_000.0).round() as i64).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, count)| *count).map(|(supply, _)| supply as f64 / 100_000.0)
}

/// The tariff of a run of days, in date order.
fn tariff(days: &[&DaySummary]) -> Option<Tariff> {
    let usage = fit(days, USAGE);
    let feed_in = fit(days, SOLAR);
    Some(Tariff {
        first: days.first()?.date?,
        last: days.last()?.date?,
        days: days.iter().filter(|day| fittable(day, USAGE)).count(),
        usage,
        feed_in,
        daily: daily(days),
    })
}

/// How a day compares with a tariff, for each category it does not fit.
pub fn check(day: &DaySummary, tariff: &Tariff) -> Vec<DayCheck> {
    let mut checks = Vec::new();
    for category in [USAGE, SOLAR] {
        let Some(charged) = day.cost.get(category).copied() else { continue };
        let Some(expected) = tariff.cost(day, category) else { continue };
        if (charged - expected).abs() > TOLERANCE {
            let meter: f64 = kwh(day, category).values().sum();
            let kwh = day.quantity.get(category).filter(|_| !kwh_agree(day, category)).map(|quantity| (*quantity, meter));
            checks.push(DayCheck { category: category.to_string(), charged, expected, kwh });
        }
    }
    checks
}

/// How many of days, which can be fitted, do not fit a tariff, the measure of how well it fits them.
fn misfits(days: &[&DaySummary], tariff: &Tariff) -> usize {
    days.iter()
        .filter(|day| fittable(day, USAGE) && fittable(day, SOLAR))
        .filter(|day| !check(day, tariff).is_empty())
        .count()
}

/// The runs of days, in date order, each with the tariff which fits it: all of them, or if a split of them into two
/// fits better, each half's runs, see the module comment.
fn split(days: &[&DaySummary]) -> Vec<Tariff> {
    let Some(whole) = tariff(days) else { return Vec::new() };
    let whole_misfits = misfits(days, &whole);
    // Too few misfits to be a change of rates rather than odd days.
    if whole_misfits < MIN_EXTRA_DAYS {
        return vec![whole];
    }
    let mut best: Option<(usize, usize)> = None;
    for at in 1..days.len() {
        let (before, after) = days.split_at(at);
        let (Some(first), Some(second)) = (tariff(before), tariff(after)) else { continue };
        // Each half must have its rates worked out, from enough days.
        let rated = |tariff: &Tariff| tariff.usage.iter().all(|rate| rate.rate.is_some()) && tariff.days >= tariff.usage.len() + MIN_EXTRA_DAYS;
        if !rated(&first) || !rated(&second) {
            continue;
        }
        let total = misfits(before, &first) + misfits(after, &second);
        if best.is_none_or(|(_, best_total)| total < best_total) {
            best = Some((at, total));
        }
    }
    match best {
        Some((at, total)) if total + MIN_EXTRA_DAYS <= whole_misfits => {
            let (before, after) = days.split_at(at);
            let mut tariffs = split(before);
            tariffs.extend(split(after));
            tariffs
        }
        _ => vec![whole],
    }
}

/// The tariffs of the days, oldest first, see the module comment.
pub fn tariffs(days: &[DaySummary]) -> Vec<Tariff> {
    let mut ordered: Vec<&DaySummary> = days.iter().filter(|day| day.date.is_some()).collect();
    ordered.sort_by_key(|day| day.date);
    split(&ordered)
}

/// The tariff of a day, from tariffs.
pub fn tariff_of<'a>(tariffs: &'a [Tariff], date: Date) -> Option<&'a Tariff> {
    tariffs.iter().find(|tariff| tariff.first <= date && date <= tariff.last)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEAK: &str = "Peak Usage";
    const OFFPEAK: &str = "Offpeak Usage";
    const FEED_IN: &str = "Standard Solar Feed in";

    /// A whole day of actual readings, of peak and offpeak kWh and kWh exported, charged at rates, rounded to the cent,
    /// as GloBird does, with the supply charge.
    fn day(date: Date, peak: f64, offpeak: f64, export: f64, rates: (f64, f64, f64), supply: f64) -> DaySummary {
        let cent = |amount: f64| (amount * 100.0).round() / 100.0;
        DaySummary {
            date: Some(date),
            import: BTreeMap::from([(PEAK.to_string(), peak), (OFFPEAK.to_string(), offpeak)]),
            export,
            export_by_type: BTreeMap::from([(FEED_IN.to_string(), export)]),
            readings: 288,
            cost: BTreeMap::from([
                (USAGE.to_string(), cent(peak * rates.0 + offpeak * rates.1)),
                (SOLAR.to_string(), cent(-export * rates.2)),
                (SUPPLY.to_string(), supply),
            ]),
            quantity: BTreeMap::from([(USAGE.to_string(), peak + offpeak), (SOLAR.to_string(), export)]),
            ..Default::default()
        }
    }

    /// Days from first, with kWh which vary from day to day, as a household's do.
    fn days(first: Date, count: i64, rates: (f64, f64, f64), supply: f64) -> Vec<DaySummary> {
        (0..count).map(|index| {
            let wobble = ((index * 7919) % 13) as f64;
            day(first + time::Duration::days(index), 0.5 + wobble * 0.4, 20.0 + wobble * 1.7 + (index % 3) as f64, 2.0 + (index % 5) as f64, rates, supply)
        }).collect()
    }

    fn rate(rates: &[Rate], charge_type: &str) -> f64 {
        rates.iter().find(|rate| rate.charge_type == charge_type).unwrap().rate.unwrap()
    }

    #[test]
    fn the_rates_are_worked_out_from_the_days_costs() {
        let days = days(time::macros::date!(2026 - 09 - 26), 30, (0.3718, 0.198, 0.03), 1.309);
        let tariffs = tariffs(&days);
        assert_eq!(tariffs.len(), 1);
        let tariff = &tariffs[0];
        assert!((rate(&tariff.usage, PEAK) - 0.3718).abs() < 0.002, "{:?}", tariff.usage);
        assert!((rate(&tariff.usage, OFFPEAK) - 0.198).abs() < 0.0005, "{:?}", tariff.usage);
        assert!((rate(&tariff.feed_in, FEED_IN) + 0.03).abs() < 0.0005, "{:?}", tariff.feed_in);
        assert_eq!(tariff.daily, Some(1.309));
        assert!(days.iter().all(|day| check(day, tariff).is_empty()));
    }

    #[test]
    fn days_whose_kwh_were_moved_are_left_out_of_the_fit_and_say_so() {
        let mut days = days(time::macros::date!(2026 - 09 - 26), 30, (0.3718, 0.198, 0.03), 1.309);
        // GloBird charges 3.5 kWh of one day's on the next.
        for (index, moved) in [(10, -3.5), (11, 3.5)] {
            let day = &mut days[index];
            *day.quantity.get_mut(USAGE).unwrap() += moved;
            *day.cost.get_mut(USAGE).unwrap() += moved * 0.198;
        }
        let tariffs = tariffs(&days);
        assert_eq!(tariffs.len(), 1);
        assert!((rate(&tariffs[0].usage, OFFPEAK) - 0.198).abs() < 0.0005);
        let checks = check(&days[10], &tariffs[0]);
        assert_eq!(checks.len(), 1);
        let (charged, meter) = checks[0].kwh.unwrap();
        assert!((meter - charged - 3.5).abs() < 1e-9);
    }

    #[test]
    fn a_change_of_rates_starts_a_new_run() {
        let mut all = days(time::macros::date!(2026 - 06 - 01), 30, (0.35, 0.19, 0.04), 1.25);
        all.extend(days(time::macros::date!(2026 - 07 - 01), 30, (0.3718, 0.198, 0.03), 1.309));
        let tariffs = tariffs(&all);
        assert_eq!(tariffs.len(), 2, "{tariffs:?}");
        assert_eq!((tariffs[0].last, tariffs[1].first), (time::macros::date!(2026 - 06 - 30), time::macros::date!(2026 - 07 - 01)));
        assert!((rate(&tariffs[0].usage, OFFPEAK) - 0.19).abs() < 0.0005);
        assert!((rate(&tariffs[1].usage, OFFPEAK) - 0.198).abs() < 0.0005);
        assert_eq!((tariffs[0].daily, tariffs[1].daily), (Some(1.25), Some(1.309)));
    }

    #[test]
    fn a_charge_type_with_too_few_kwh_has_no_rate() {
        let mut days = days(time::macros::date!(2026 - 09 - 26), 10, (0.3718, 0.198, 0.03), 1.309);
        for day in &mut days {
            let offpeak = day.import[OFFPEAK];
            day.import.insert(PEAK.to_string(), 0.01);
            day.cost.insert(USAGE.to_string(), ((0.01 * 0.3718 + offpeak * 0.198) * 100.0).round() / 100.0);
            day.quantity.insert(USAGE.to_string(), 0.01 + offpeak);
        }
        let tariff = &tariffs(&days)[0];
        assert_eq!(tariff.usage.iter().find(|rate| rate.charge_type == PEAK).unwrap().rate, None);
        assert!((rate(&tariff.usage, OFFPEAK) - 0.198).abs() < 0.001);
    }
}
