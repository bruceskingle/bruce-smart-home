//! Checking a site's topology against what its channels measured, see power::topology, and what the measurements say of
//! where the channels are, whatever the topology, for the Site page. Each channel's net import, what went into its
//! circuit less what came out, e.g. a battery's charge less its discharge, or the solar panels' less what they made, is
//! added up each half hour. Energy is conserved, so a circuit's net import is the sum of the circuits it feeds'.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::measured::{HALF_HOUR, Measurements};
use super::topology::Circuit;

/// Each channel's net import each half hour, in kWh, None where it was not measured all through it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    /// The start of each half hour, as unix timestamps.
    pub half_hours: Vec<i64>,
    /// By channel id, a value for each half hour, including the grid readings' as grid_name.
    pub values: BTreeMap<String, Vec<Option<f64>>>,
    /// The channel the grid readings are of, e.g. octopus:electricity, which no device module measures, see
    /// Series::new.
    pub grid_name: String,
}

impl Series {
    /// The half hours from start to end, with the grid readings' net import of each, grid, by the half hour's start,
    /// as grid_name, and each channel of measurements'.
    pub fn new(
        start: i64,
        end: i64,
        grid: &BTreeMap<i64, f64>,
        grid_name: &str,
        measurements: &[Arc<dyn Measurements>],
    ) -> Series {
        let first = start.div_euclid(HALF_HOUR) * HALF_HOUR;
        let half_hours: Vec<i64> = (first..end)
            .step_by(HALF_HOUR as usize)
            .filter(|half_hour| half_hour + HALF_HOUR <= end)
            .collect();
        let mut values = BTreeMap::new();
        if !grid.is_empty() {
            values.insert(
                grid_name.to_string(),
                half_hours
                    .iter()
                    .map(|half_hour| grid.get(half_hour).copied())
                    .collect(),
            );
        }
        for measurements in measurements {
            let complete: Vec<bool> = half_hours
                .iter()
                .map(|half_hour| measurements.complete(*half_hour))
                .collect();
            for channel in measurements.channels() {
                let channel_values: Vec<Option<f64>> = half_hours
                    .iter()
                    .zip(&complete)
                    .map(|(half_hour, complete)| {
                        let flow = measurements
                            .energy(&channel, *half_hour, half_hour + HALF_HOUR)
                            .filter(|_| *complete)?;
                        Some(flow.import - flow.export)
                    })
                    .collect();
                if channel_values.iter().any(Option::is_some) {
                    values.insert(channel, channel_values);
                }
            }
        }
        Series {
            half_hours,
            values,
            grid_name: grid_name.to_string(),
        }
    }

    /// The series of a circuit: its first channel which has one, or the grid readings', if it is the grid's.
    fn of(&self, circuit: &Circuit, top: bool) -> Option<(&str, &[Option<f64>])> {
        circuit
            .channels
            .iter()
            .find_map(|channel| self.values.get_key_value(channel))
            .or_else(|| top.then(|| self.values.get_key_value(&self.grid_name)).flatten())
            .map(|(name, values)| (name.as_str(), values.as_slice()))
    }

    /// Each half hour's start, and its values of the series, where every one of them has a value.
    fn common<'a>(&'a self, series: &[&'a [Option<f64>]]) -> impl Iterator<Item = (i64, Vec<f64>)> + 'a {
        let series = series.to_vec();
        self.half_hours
            .iter()
            .enumerate()
            .filter_map(move |(index, half_hour)| {
                Some((
                    *half_hour,
                    series
                        .iter()
                        .map(|values| values[index])
                        .collect::<Option<Vec<f64>>>()?,
                ))
            })
    }
}

/// How far out a circuit's net import of a half hour can be from what it should be before it says so, in kWh.
const NOISE: f64 = 0.05;
/// How many of the half hours furthest out a check shows.
const WORST: usize = 3;

/// What a check compares, see check.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckKind {
    /// A measured circuit, and the circuits it feeds, all measured, which should add up to it.
    AddsUp,
    /// Two channels on one circuit, which should agree.
    Agrees,
    /// A circuit without a channel, the rest of a measured circuit's net import, which should not be less than none.
    Rest,
}

/// A check of the topology over the half hours which have everything it compares: what it is of, e.g. Grid feeds Car
/// and Gateway, and the totals of the two sides, e.g. the grid's and the sum of the car's and the gateway's, in kWh.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub kind: CheckKind,
    pub title: String,
    pub left: String,
    pub right: String,
    pub half_hours: usize,
    pub left_total: f64,
    pub right_total: f64,
    /// How far out the sides are, the sum of each half hour's difference, whichever way it is, as a share of the sum of
    /// the left side's, e.g. 0.01 for 1%. For a Rest, which has no right side to agree with, None.
    pub mismatch: Option<f64>,
    /// How many half hours the rest was less than none, beyond NOISE.
    pub negative: usize,
    /// The half hours furthest out, with each side's value, furthest first, or for a Rest, the lowest.
    pub worst: Vec<(i64, f64, f64)>,
}

impl Check {
    fn new(kind: CheckKind, title: String, left: String, right: String, pairs: Vec<(i64, f64, f64)>) -> Check {
        let left_total: f64 = pairs.iter().map(|(_, left, _)| left).sum();
        let right_total: f64 = pairs.iter().map(|(_, _, right)| right).sum();
        let magnitude: f64 = pairs.iter().map(|(_, left, _)| left.abs()).sum();
        let difference: f64 = pairs.iter().map(|(_, left, right)| (left - right).abs()).sum();
        let mut worst = pairs.clone();
        if kind == CheckKind::Rest {
            worst.sort_by(|a, b| (a.1 - a.2).total_cmp(&(b.1 - b.2)));
            worst.retain(|(_, left, right)| left - right < -NOISE);
        } else {
            worst.sort_by(|a, b| (b.1 - b.2).abs().total_cmp(&(a.1 - a.2).abs()));
        }
        worst.truncate(WORST);
        Check {
            kind,
            title,
            left,
            right,
            half_hours: pairs.len(),
            left_total,
            right_total,
            mismatch: (kind != CheckKind::Rest && magnitude > 0.0).then(|| difference / magnitude),
            negative: pairs.iter().filter(|(_, left, right)| left - right < -NOISE).count(),
            worst,
        }
    }
}

/// The checks of topology against series, see the module comment and CheckKind.
pub fn checks(topology: &Circuit, series: &Series) -> Vec<Check> {
    let mut checks = Vec::new();
    add_checks(topology, true, series, &mut checks);
    checks
}

fn add_checks(circuit: &Circuit, top: bool, series: &Series, checks: &mut Vec<Check>) {
    // Each pair of channels on it, the first against each other.
    let measured: Vec<(&String, &Vec<Option<f64>>)> = circuit
        .channels
        .iter()
        .filter_map(|channel| series.values.get_key_value(channel))
        .collect();
    let first = series.of(circuit, top);
    if let Some((first_name, first_values)) = first {
        for (name, values) in measured.iter().filter(|(name, _)| name.as_str() != first_name) {
            let pairs = series
                .common(&[first_values, values])
                .map(|(half_hour, values)| (half_hour, values[0], values[1]))
                .collect();
            checks.push(Check::new(
                CheckKind::Agrees,
                format!("{} is measured by {first_name} and {name}", circuit.name),
                first_name.to_string(),
                name.to_string(),
                pairs,
            ));
        }
    }
    if let (Some((parent_name, parent_values)), false) = (first, circuit.children.is_empty()) {
        let children: Vec<(&Circuit, Option<(&str, &[Option<f64>])>)> = circuit
            .children
            .iter()
            .map(|child| (child, series.of(child, false)))
            .collect();
        let unmeasured: Vec<&Circuit> = children
            .iter()
            .filter(|(_, series)| series.is_none())
            .map(|(child, _)| *child)
            .collect();
        let measured: Vec<(&Circuit, &str, &[Option<f64>])> = children
            .iter()
            .filter_map(|(child, series)| series.map(|(name, values)| (*child, name, values)))
            .collect();
        let mut all = vec![parent_values];
        all.extend(measured.iter().map(|(_, _, values)| *values));
        let pairs: Vec<(i64, f64, f64)> = series
            .common(&all)
            .map(|(half_hour, values)| (half_hour, values[0], values[1..].iter().sum()))
            .collect();
        let names = |circuits: &[&Circuit]| {
            circuits
                .iter()
                .map(|circuit| circuit.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let measured_names = measured
            .iter()
            .map(|(_, name, _)| *name)
            .collect::<Vec<_>>()
            .join(" + ");
        match unmeasured.as_slice() {
            [] => checks.push(Check::new(
                CheckKind::AddsUp,
                format!(
                    "{} feeds {}",
                    circuit.name,
                    names(&measured.iter().map(|(child, _, _)| *child).collect::<Vec<_>>())
                ),
                parent_name.to_string(),
                measured_names,
                pairs,
            )),
            [rest] => checks.push(Check::new(
                CheckKind::Rest,
                format!("{}, the rest of {}", rest.name, circuit.name),
                parent_name.to_string(),
                if measured_names.is_empty() {
                    String::from("nothing")
                } else {
                    measured_names
                },
                pairs,
            )),
            _ => {}
        }
    }
    for child in &circuit.children {
        add_checks(child, false, series, checks);
    }
}

/// How far out an AddsUp or Agrees check can be for the topology to look right, see verdict: more than the meters'
/// usual disagreement of a few percent.
const RIGHT: f64 = 0.10;
/// The share of a Rest's half hours which can be less than none for the topology to look right, as the meters are
/// read at slightly different moments.
const RIGHT_NEGATIVE: f64 = 0.02;

/// Whether a topology looks right, from its checks.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// There is nothing to check, e.g. no circuit has its own channel and those of all the circuits it feeds.
    NothingToCheck,
    /// Every check is within RIGHT, and every rest less than none in no more than RIGHT_NEGATIVE of its half hours.
    LooksRight,
    /// The indexes of the checks which are not.
    Wrong(Vec<usize>),
}

/// Whether the checks of a topology look right, see Verdict.
pub fn verdict(checks: &[Check]) -> Verdict {
    if checks.iter().all(|check| check.half_hours == 0) {
        return Verdict::NothingToCheck;
    }
    let wrong: Vec<usize> = checks
        .iter()
        .enumerate()
        .filter(|(_, check)| {
            check.half_hours > 0
                && match check.kind {
                    CheckKind::Rest => check.negative as f64 > check.half_hours as f64 * RIGHT_NEGATIVE,
                    _ => check.mismatch.is_none_or(|mismatch| mismatch > RIGHT),
                }
        })
        .map(|(index, _)| index)
        .collect();
    if wrong.is_empty() {
        Verdict::LooksRight
    } else {
        Verdict::Wrong(wrong)
    }
}

/// A topology from what the measurements say, see relations, None if they say nothing of the grid: channels which are
/// the same are one circuit, and a circuit whose channel is the sum of others' feeds theirs, each being fed from the
/// smallest circuit it is part of the sum of. A channel the data does not place is fed from the grid. A circuit whose
/// circuits are not all of a sum also feeds the rest, and one which is the house's, e.g. a gateway's load, which it
/// works out from its other meters, is the rest itself. The names and uses of current's circuits are kept for the
/// same channels, and others' are from what the channels usually measure, see Channel::use_.
pub fn propose(
    current: &Circuit,
    series: &Series,
    relations: &[Relation],
    channels: &[super::topology::Channel],
) -> Option<Circuit> {
    use super::topology::Use;
    let names: Vec<&str> = series.values.keys().map(String::as_str).collect();
    // The first of each group of channels which are the same, see relations.
    let mut group: BTreeMap<&str, &str> = names.iter().map(|name| (*name, *name)).collect();
    for relation in relations {
        if let Relation::Same { a, b, .. } = relation {
            let first = group[a.as_str()];
            group.insert(b.as_str(), first);
        }
    }
    let root = *group.get(series.grid_name.as_str())?;
    let firsts: Vec<&str> = names.iter().copied().filter(|name| group[name] == *name).collect();
    let magnitude = |name: &str| {
        series.values[name]
            .iter()
            .flatten()
            .map(|value| value.abs())
            .sum::<f64>()
    };
    // What feeds each, the smallest sum it is part of, and the sums.
    let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
    let mut sums: Vec<(&str, BTreeSet<&str>)> = relations
        .iter()
        .filter_map(|relation| match relation {
            Relation::Sum { a, parts, .. } => Some((
                group[a.as_str()],
                parts.iter().map(|part| group[part.as_str()]).collect(),
            )),
            _ => None,
        })
        .collect();
    sums.sort_by(|a, b| magnitude(a.0).total_cmp(&magnitude(b.0)));
    /// name and what feeds it, and what feeds that, up to the grid.
    fn ancestors<'a>(parent: &BTreeMap<&'a str, &'a str>, mut name: &'a str) -> Vec<&'a str> {
        let mut ancestors = vec![name];
        while let Some(next) = parent.get(name) {
            name = next;
            ancestors.push(name);
        }
        ancestors
    }
    for (a, parts) in &sums {
        for part in parts {
            if *part != root && !parent.contains_key(part) && !ancestors(&parent, a).contains(part) {
                parent.insert(part, a);
            }
        }
    }
    for first in &firsts {
        if *first != root && !parent.contains_key(first) {
            parent.insert(first, root);
        }
    }
    // Whether a circuit's are just those of one of its sums, so that it needs no rest.
    let exact = |name: &str| -> bool {
        let fed: BTreeSet<&str> = parent
            .iter()
            .filter(|(_, feeder)| **feeder == name)
            .map(|(fed, _)| *fed)
            .collect();
        sums.iter().any(|(a, parts)| *a == name && *parts == fed)
    };
    let use_of = |id: &str| {
        channels
            .iter()
            .find(|channel| channel.id == id)
            .map(|channel| channel.use_.clone())
    };
    let current_of = |members: &[&str]| -> Option<Circuit> {
        fn find<'a>(circuit: &'a Circuit, members: &[&str]) -> Option<&'a Circuit> {
            if circuit
                .channels
                .iter()
                .any(|channel| members.contains(&channel.as_str()))
            {
                return Some(circuit);
            }
            circuit.children.iter().find_map(|child| find(child, members))
        }
        find(current, members).cloned()
    };
    fn build(
        name: &str,
        top: bool,
        group: &BTreeMap<&str, &str>,
        parent: &BTreeMap<&str, &str>,
        exact: &dyn Fn(&str) -> bool,
        use_of: &dyn Fn(&str) -> Option<Use>,
        current_of: &dyn Fn(&[&str]) -> Option<Circuit>,
    ) -> Circuit {
        let mut members: Vec<&str> = group
            .iter()
            .filter(|(_, first)| **first == name)
            .map(|(member, _)| *member)
            .collect();
        // Not the grid's circuit, for one fed from it, e.g. where the current topology has a clamp at the grid
        // connection which the data says is not.
        let current = current_of(&members).filter(|current| top || current.use_ != Use::Grid);
        // In the order the current topology has them, as the first is the one used.
        if let Some(current) = &current {
            members.sort_by_key(|member| {
                current
                    .channels
                    .iter()
                    .position(|channel| channel == member)
                    .unwrap_or(usize::MAX)
            });
        }
        let mut children: Vec<Circuit> = parent
            .iter()
            .filter(|(_, feeder)| **feeder == name)
            .map(|(fed, _)| build(fed, false, group, parent, exact, use_of, current_of))
            .collect();
        let use_ = if top {
            Use::Grid
        } else if !children.is_empty() {
            Use::Board
        } else {
            current
                .as_ref()
                .map(|current| current.use_.clone())
                .or_else(|| members.iter().find_map(|member| use_of(member)))
                .unwrap_or_default()
        };
        // A clamp which would be the grid's, inside the site, e.g. a zappi's grid clamp moved beside a gateway's, is a
        // board's, which feeds the rest.
        let use_ = if !top && use_ == Use::Grid { Use::Board } else { use_ };
        // Then those which usually measure what it is for first, e.g. a gateway's site meter for a board, unless the
        // current topology has them in another order.
        if current.is_none() {
            members.sort_by_key(|member| use_of(member).as_ref() != Some(&use_));
        }
        if (if children.is_empty() {
            use_ == Use::Board
        } else {
            !exact(name)
        }) && children.iter().all(|child| !child.channels.is_empty())
        {
            children.push(Circuit {
                name: String::from(super::topology::UNKNOWN),
                use_: Use::unknown(),
                channels: Vec::new(),
                children: Vec::new(),
            });
        }
        // The house's channel in a sum is the rest of what feeds it.
        if exact(name) {
            let houses: Vec<usize> = children
                .iter()
                .enumerate()
                .filter(|(_, child)| child.use_ == Use::house() && child.children.is_empty())
                .map(|(index, _)| index)
                .collect();
            if let [house] = houses.as_slice() {
                children[*house].channels.clear();
            }
        }
        let name = current
            .as_ref()
            .map(|current| current.name.clone())
            .unwrap_or_else(|| match &use_ {
                Use::Board => String::from("Board"),
                other => other.label().to_string(),
            });
        Circuit {
            name,
            use_,
            channels: members.iter().map(|member| member.to_string()).collect(),
            children,
        }
    }
    Some(build(root, true, &group, &parent, &exact, &use_of, &current_of))
}

/// How far out two series can be to be taken as one, see Relation.
const SAME: f64 = 0.05;
/// The least a series's net import can add up to, whichever way, over the half hours compared, for it to be compared,
/// in kWh, as any two which are about none agree.
const LEAST: f64 = 1.0;
/// The fewest half hours to compare.
const FEWEST: usize = 48;
/// The most series a sum can have, e.g. a gateway at the grid connection feeding the car, the batteries, the solar
/// panels and the house.
const MOST_PARTS: usize = 4;

/// What the measurements say of how two or more channels relate, whatever the topology, so that the user can place
/// them: each mismatch is as Check::mismatch.
#[derive(Clone, Debug, PartialEq)]
pub enum Relation {
    /// Both measure the same circuit, e.g. two clamps beside each other.
    Same {
        a: String,
        b: String,
        mismatch: f64,
        half_hours: usize,
    },
    /// b measures what a does, the other way round, e.g. a clamp put on backwards.
    Reversed {
        a: String,
        b: String,
        mismatch: f64,
        half_hours: usize,
    },
    /// a's net import is the sum of the parts', as a circuit feeding them would be.
    Sum {
        a: String,
        parts: Vec<String>,
        mismatch: f64,
        half_hours: usize,
    },
}

/// How far a is from the sum of parts, each times its sign, over the half hours which have them all, None if there
/// are too few, or a or a part is too small to say, see LEAST.
fn mismatch(series: &Series, a: &str, parts: &[(&str, f64)]) -> Option<(f64, usize)> {
    let mut all = vec![series.values.get(a)?.as_slice()];
    for (part, _) in parts {
        all.push(series.values.get(*part)?.as_slice());
    }
    let rows: Vec<Vec<f64>> = series.common(&all).map(|(_, values)| values).collect();
    if rows.len() < FEWEST {
        return None;
    }
    let magnitude: f64 = rows.iter().map(|row| row[0].abs()).sum();
    // Each part must matter, or a sum with a part of nothing would be found as well as a without it.
    if magnitude < LEAST
        || (1..all.len()).any(|index| rows.iter().map(|row| row[index].abs()).sum::<f64>() < magnitude * SAME)
    {
        return None;
    }
    let difference: f64 = rows
        .iter()
        .map(|row| {
            (row[0]
                - parts
                    .iter()
                    .zip(&row[1..])
                    .map(|((_, sign), value)| sign * value)
                    .sum::<f64>())
            .abs()
        })
        .sum();
    Some((difference / magnitude, rows.len()))
}

/// The relations between the series's channels, see Relation: those which are the same first, then the sums, of only
/// one of each group of the same, with no part which another sum of fewer parts already has, and none which only
/// spells out a part of another.
pub fn relations(series: &Series) -> Vec<Relation> {
    let names: Vec<&str> = series.values.keys().map(String::as_str).collect();
    let mut relations = Vec::new();
    // The first of each group of series which are the same, which stands for the others in sums.
    let mut same_as: BTreeMap<&str, &str> = BTreeMap::new();
    for (index, a) in names.iter().enumerate() {
        for b in &names[index + 1..] {
            if let Some((mismatch, half_hours)) =
                mismatch(series, a, &[(b, 1.0)]).filter(|(mismatch, _)| *mismatch <= SAME)
            {
                relations.push(Relation::Same {
                    a: a.to_string(),
                    b: b.to_string(),
                    mismatch,
                    half_hours,
                });
                let first = same_as.get(a).copied().unwrap_or(a);
                same_as.entry(b).or_insert(first);
            } else if let Some((mismatch, half_hours)) =
                mismatch(series, a, &[(b, -1.0)]).filter(|(mismatch, _)| *mismatch <= SAME)
            {
                relations.push(Relation::Reversed {
                    a: a.to_string(),
                    b: b.to_string(),
                    mismatch,
                    half_hours,
                });
                // Nor in sums, where it would be the other way round, e.g. the car's the grid's and the reversed one's.
                let first = same_as.get(a).copied().unwrap_or(a);
                same_as.entry(b).or_insert(first);
            }
        }
    }
    let firsts: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !same_as.contains_key(name))
        .collect();
    for a in &firsts {
        let others: Vec<&str> = firsts.iter().copied().filter(|other| other != a).collect();
        let mut found: Vec<BTreeSet<&str>> = Vec::new();
        for size in 2..=MOST_PARTS.min(others.len()) {
            for parts in combinations(&others, size) {
                let set: BTreeSet<&str> = parts.iter().copied().collect();
                if found.iter().any(|smaller| smaller.is_subset(&set)) {
                    continue;
                }
                let signed: Vec<(&str, f64)> = parts.iter().map(|part| (*part, 1.0)).collect();
                if let Some((mismatch, half_hours)) =
                    mismatch(series, a, &signed).filter(|(mismatch, _)| *mismatch <= SAME)
                {
                    relations.push(Relation::Sum {
                        a: a.to_string(),
                        parts: parts.iter().map(|part| part.to_string()).collect(),
                        mismatch,
                        half_hours,
                    });
                    found.push(set);
                }
            }
        }
    }
    // Not a sum which is another's with a part replaced by that part's own sum, e.g. the grid's as the car's, the
    // batteries', the solar panels' and the load's, where it is the car's and the gateway's, and the gateway's those.
    let sums: Vec<(String, BTreeSet<String>)> = relations
        .iter()
        .filter_map(|relation| match relation {
            Relation::Sum { a, parts, .. } => Some((a.clone(), parts.iter().cloned().collect())),
            _ => None,
        })
        .collect();
    let expanded = |a: &str, parts: &BTreeSet<String>| {
        sums.iter().any(|(other_a, other_parts)| {
            other_a == a
                && other_parts.iter().any(|part| {
                    sums.iter().any(|(sum_a, sum_parts)| {
                        sum_a == part && {
                            let mut replaced: BTreeSet<String> =
                                other_parts.iter().filter(|other| *other != part).cloned().collect();
                            replaced.extend(sum_parts.iter().cloned());
                            replaced == *parts
                        }
                    })
                })
        })
    };
    relations.retain(
        |relation| !matches!(relation, Relation::Sum { a, parts, .. } if expanded(a, &parts.iter().cloned().collect())),
    );
    relations
}

/// Every choice of size of items, in order.
fn combinations<'a>(items: &[&'a str], size: usize) -> Vec<Vec<&'a str>> {
    if size == 0 {
        return vec![Vec::new()];
    }
    (0..items.len())
        .flat_map(|index| {
            combinations(&items[index + 1..], size - 1)
                .into_iter()
                .map(move |mut rest| {
                    rest.insert(0, items[index]);
                    rest
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::topology::{Channel, Use};

    const DAY: i64 = 1_793_232_000;

    /// A day of half hours: the car charging 3.5 kWh a half hour for the first 8, the gateway's site meter 0.5 kWh
    /// each, plus a little which varies, so that the series are not all the same shape, the zappi's grid clamp beside
    /// the gateway's, and the grid meter the car's and the gateway's together.
    fn series() -> Series {
        let half_hours: Vec<i64> = (0..48).map(|index| DAY + index * HALF_HOUR).collect();
        let car: Vec<f64> = (0..48).map(|index| if index < 8 { 3.5 } else { 0.0 }).collect();
        let site: Vec<f64> = (0..48).map(|index| 0.5 + (index % 5) as f64 * 0.1).collect();
        let grid: Vec<f64> = car.iter().zip(&site).map(|(car, site)| car + site).collect();
        let some = |values: &[f64]| values.iter().map(|value| Some(*value)).collect::<Vec<_>>();
        let values = BTreeMap::from([
            (String::from("octopus:electricity"), some(&grid)),
            (String::from("myenergi:Z1"), some(&car)),
            (String::from("myenergi:Z1:grid"), some(&site)),
            (String::from("tesla:site"), some(&site)),
        ]);
        Series {
            half_hours,
            values,
            grid_name: String::from("octopus:electricity"),
        }
    }

    fn circuit(name: &str, use_: Use, channels: &[&str], children: Vec<Circuit>) -> Circuit {
        Circuit {
            name: name.to_string(),
            use_,
            channels: channels.iter().map(|channel| channel.to_string()).collect(),
            children,
        }
    }

    #[test]
    fn the_first_households_topology_adds_up_and_its_two_clamps_agree() {
        let topology = circuit(
            "Grid",
            Use::Grid,
            &["octopus:electricity"],
            vec![
                circuit("Car", Use::car(), &["myenergi:Z1"], vec![]),
                circuit(
                    "Gateway",
                    Use::Board,
                    &["tesla:site", "myenergi:Z1:grid"],
                    vec![circuit("House", Use::house(), &[], vec![])],
                ),
            ],
        );
        let checks = checks(&topology, &series());
        let kinds: Vec<CheckKind> = checks.iter().map(|check| check.kind).collect();
        assert_eq!(
            kinds,
            [CheckKind::AddsUp, CheckKind::Agrees, CheckKind::Rest],
            "{checks:#?}"
        );
        assert!(checks[0].mismatch.unwrap() < 1e-9 && checks[0].half_hours == 48);
        assert_eq!(checks[0].title, "Grid feeds Car, Gateway");
        assert!(checks[1].mismatch.unwrap() < 1e-9);
        assert_eq!(checks[2].negative, 0);
    }

    #[test]
    fn with_the_car_put_behind_the_gateway_the_checks_say_so() {
        // As installers fit it, which this house is not: the grid is then the gateway's, and the car is more than the
        // rest of the gateway's.
        let topology = circuit(
            "Grid",
            Use::Grid,
            &["octopus:electricity", "tesla:site"],
            vec![
                circuit("Car", Use::car(), &["myenergi:Z1"], vec![]),
                circuit("House", Use::house(), &[], vec![]),
            ],
        );
        let checks = checks(&topology, &series());
        let agrees = checks.iter().find(|check| check.kind == CheckKind::Agrees).unwrap();
        assert!(agrees.mismatch.unwrap() > 0.3, "{agrees:?}");
        let mut topology = topology;
        topology.channels = vec![String::from("tesla:site")];
        let rest = checks_of(&topology)
            .into_iter()
            .find(|check| check.kind == CheckKind::Rest)
            .unwrap();
        assert_eq!(rest.negative, 8, "the car's 8 half hours of charging");
        assert_eq!(rest.worst.len(), WORST);
    }

    fn checks_of(topology: &Circuit) -> Vec<Check> {
        checks(topology, &series())
    }

    #[test]
    fn the_data_says_which_clamps_are_the_same_and_what_adds_up() {
        let mut series = series();
        // A clamp on backwards.
        let reversed: Vec<Option<f64>> = series.values["tesla:site"]
            .iter()
            .map(|value| value.map(|value| -value))
            .collect();
        series.values.insert(String::from("other:backwards"), reversed);
        let relations = relations(&series);
        let same = |a: &str, b: &str| {
            relations
                .iter()
                .any(|relation| matches!(relation, Relation::Same { a: x, b: y, .. } if x == a && y == b))
        };
        assert!(same("myenergi:Z1:grid", "tesla:site"), "{relations:#?}");
        assert!(relations.iter().any(|relation| matches!(relation, Relation::Reversed { a, b, .. } if a == "myenergi:Z1:grid" && b == "other:backwards")));
        let sums: Vec<(&str, Vec<&str>)> = relations
            .iter()
            .filter_map(|relation| match relation {
                Relation::Sum { a, parts, .. } => Some((a.as_str(), parts.iter().map(String::as_str).collect())),
                _ => None,
            })
            .collect();
        assert_eq!(sums, [("octopus:electricity", vec!["myenergi:Z1", "myenergi:Z1:grid"])]);
    }

    #[test]
    fn the_right_topology_looks_right_and_a_wrong_one_gets_one_which_fits() {
        let series = series();
        let right = circuit(
            "Grid",
            Use::Grid,
            &["octopus:electricity"],
            vec![
                circuit("Car", Use::car(), &["myenergi:Z1"], vec![]),
                circuit(
                    "Gateway",
                    Use::Board,
                    &["tesla:site", "myenergi:Z1:grid"],
                    vec![circuit("House", Use::house(), &[], vec![])],
                ),
            ],
        );
        assert_eq!(verdict(&checks(&right, &series)), Verdict::LooksRight);
        // As installers fit it, which this house is not.
        let wrong = circuit(
            "Grid",
            Use::Grid,
            &["octopus:electricity", "tesla:site", "myenergi:Z1:grid"],
            vec![
                circuit("Car", Use::car(), &["myenergi:Z1"], vec![]),
                circuit("House", Use::house(), &[], vec![]),
            ],
        );
        assert!(matches!(verdict(&checks(&wrong, &series)), Verdict::Wrong(_)));
        let channels: Vec<Channel> = [
            ("octopus:electricity", Use::Grid),
            ("myenergi:Z1", Use::car()),
            ("myenergi:Z1:grid", Use::Grid),
            ("tesla:site", Use::Board),
        ]
        .map(|(id, use_)| Channel {
            id: id.to_string(),
            description: String::new(),
            use_,
        })
        .to_vec();
        let proposed = propose(&wrong, &series, &relations(&series), &channels).unwrap();
        let expected = circuit(
            "Grid",
            Use::Grid,
            &["octopus:electricity"],
            vec![
                circuit("Car", Use::car(), &["myenergi:Z1"], vec![]),
                circuit(
                    "Board",
                    Use::Board,
                    &["tesla:site", "myenergi:Z1:grid"],
                    vec![circuit("Unknown", Use::unknown(), &[], vec![])],
                ),
            ],
        );
        assert_eq!(proposed, expected);
        assert_eq!(verdict(&checks(&proposed, &series)), Verdict::LooksRight);
        assert!(proposed.problems().is_empty());
    }

    #[test]
    fn a_house_wired_as_installers_do_gets_the_car_behind_the_gateway() {
        // The gateway's and the zappi's clamps both at the grid connection, measuring the car, the batteries, the
        // solar panels and the house, which the gateway's load is.
        let half_hours: Vec<i64> = (0..48).map(|index| DAY + index * HALF_HOUR).collect();
        let car: Vec<f64> = (0..48).map(|index| if index < 8 { 3.5 } else { 0.0 }).collect();
        let battery: Vec<f64> = (0..48)
            .map(|index| {
                if index < 10 {
                    2.0
                } else if index > 36 {
                    -0.4
                } else {
                    0.0
                }
            })
            .collect();
        let solar: Vec<f64> = (0..48)
            .map(|index| {
                if (16..32).contains(&index) {
                    -1.0 - (index % 3) as f64 * 0.2
                } else {
                    0.0
                }
            })
            .collect();
        let load: Vec<f64> = (0..48).map(|index| 0.3 + (index % 5) as f64 * 0.1).collect();
        let grid: Vec<f64> = (0..48)
            .map(|index| car[index] + battery[index] + solar[index] + load[index])
            .collect();
        let some = |values: &[f64]| values.iter().map(|value| Some(*value)).collect::<Vec<_>>();
        let series = Series {
            half_hours,
            values: BTreeMap::from([
                (String::from("octopus:electricity"), some(&grid)),
                (String::from("myenergi:Z1"), some(&car)),
                (String::from("myenergi:Z1:grid"), some(&grid)),
                (String::from("tesla:site"), some(&grid)),
                (String::from("tesla:battery"), some(&battery)),
                (String::from("tesla:solar"), some(&solar)),
                (String::from("tesla:load"), some(&load)),
            ]),
            grid_name: String::from("octopus:electricity"),
        };
        let relations = relations(&series);
        let sums: Vec<&Relation> = relations
            .iter()
            .filter(|relation| matches!(relation, Relation::Sum { .. }))
            .collect();
        assert_eq!(sums.len(), 1, "{relations:#?}");
        // The first household's topology, which this house is not.
        let beside = circuit(
            "Grid",
            Use::Grid,
            &["octopus:electricity"],
            vec![
                circuit("Car", Use::car(), &["myenergi:Z1"], vec![]),
                circuit(
                    "Gateway",
                    Use::Board,
                    &["tesla:site"],
                    vec![
                        circuit("Battery", Use::Battery, &["tesla:battery"], vec![]),
                        circuit("Solar", Use::Solar, &["tesla:solar"], vec![]),
                        circuit("House", Use::house(), &[], vec![]),
                    ],
                ),
            ],
        );
        assert!(matches!(verdict(&checks(&beside, &series)), Verdict::Wrong(_)));
        let channels: Vec<Channel> = [
            ("octopus:electricity", Use::Grid),
            ("myenergi:Z1", Use::car()),
            ("myenergi:Z1:grid", Use::Grid),
            ("tesla:site", Use::Board),
            ("tesla:battery", Use::Battery),
            ("tesla:solar", Use::Solar),
            ("tesla:load", Use::house()),
        ]
        .map(|(id, use_)| Channel {
            id: id.to_string(),
            description: String::new(),
            use_,
        })
        .to_vec();
        let proposed = propose(&beside, &series, &relations, &channels).unwrap();
        assert_eq!(
            proposed.channels,
            ["octopus:electricity", "myenergi:Z1:grid", "tesla:site"]
        );
        let fed: Vec<(&str, Use, usize)> = proposed
            .children
            .iter()
            .map(|child| (child.name.as_str(), child.use_.clone(), child.channels.len()))
            .collect();
        assert_eq!(
            fed,
            [
                ("Car", Use::car(), 1),
                ("Battery", Use::Battery, 1),
                ("House", Use::house(), 0),
                ("Solar", Use::Solar, 1)
            ]
        );
        assert_eq!(verdict(&checks(&proposed, &series)), Verdict::LooksRight);
        assert!(proposed.problems().is_empty());
    }

    #[test]
    fn a_gateway_whose_meters_add_up_feeds_its_load_as_the_rest_and_no_other() {
        // The first household: the car beside the gateway, whose load is its site less its batteries and solar.
        let half_hours: Vec<i64> = (0..48).map(|index| DAY + index * HALF_HOUR).collect();
        let car: Vec<f64> = (0..48).map(|index| if index < 8 { 3.5 } else { 0.0 }).collect();
        let battery: Vec<f64> = (0..48)
            .map(|index| {
                if index < 10 {
                    2.0
                } else if index > 36 {
                    -0.4
                } else {
                    0.0
                }
            })
            .collect();
        let solar: Vec<f64> = (0..48)
            .map(|index| {
                if (16..32).contains(&index) {
                    -1.0 - (index % 3) as f64 * 0.2
                } else {
                    0.0
                }
            })
            .collect();
        let load: Vec<f64> = (0..48).map(|index| 0.3 + (index % 5) as f64 * 0.1).collect();
        let site: Vec<f64> = (0..48)
            .map(|index| battery[index] + solar[index] + load[index])
            .collect();
        let grid: Vec<f64> = (0..48).map(|index| car[index] + site[index]).collect();
        let some = |values: &[f64]| values.iter().map(|value| Some(*value)).collect::<Vec<_>>();
        let series = Series {
            half_hours,
            values: BTreeMap::from([
                (String::from("octopus:electricity"), some(&grid)),
                (String::from("myenergi:Z1"), some(&car)),
                (String::from("myenergi:Z1:grid"), some(&site)),
                (String::from("tesla:site"), some(&site)),
                (String::from("tesla:battery"), some(&battery)),
                (String::from("tesla:solar"), some(&solar)),
                (String::from("tesla:load"), some(&load)),
            ]),
            grid_name: String::from("octopus:electricity"),
        };
        // As installers fit it, which this house is not.
        let behind = circuit(
            "Grid",
            Use::Grid,
            &["octopus:electricity", "myenergi:Z1:grid"],
            vec![circuit(
                "Gateway",
                Use::Board,
                &["tesla:site"],
                vec![
                    circuit("Car", Use::car(), &["myenergi:Z1"], vec![]),
                    circuit("House", Use::house(), &[], vec![]),
                ],
            )],
        );
        let channels: Vec<Channel> = [
            ("octopus:electricity", Use::Grid),
            ("myenergi:Z1", Use::car()),
            ("myenergi:Z1:grid", Use::Grid),
            ("tesla:site", Use::Board),
            ("tesla:battery", Use::Battery),
            ("tesla:solar", Use::Solar),
            ("tesla:load", Use::house()),
        ]
        .map(|(id, use_)| Channel {
            id: id.to_string(),
            description: String::new(),
            use_,
        })
        .to_vec();
        let proposed = propose(&behind, &series, &relations(&series), &channels).unwrap();
        let gateway = proposed.children.iter().find(|child| child.use_ == Use::Board).unwrap();
        let houses: Vec<&Circuit> = gateway
            .children
            .iter()
            .filter(|child| child.use_ == Use::house())
            .collect();
        assert_eq!(houses.len(), 1, "{proposed:#?}");
        assert!(houses[0].channels.is_empty());
        assert_eq!(verdict(&checks(&proposed, &series)), Verdict::LooksRight);
    }

    #[test]
    fn combinations_are_in_order() {
        assert_eq!(
            combinations(&["a", "b", "c"], 2),
            [vec!["a", "b"], vec!["a", "c"], vec!["b", "c"]]
        );
    }
}
