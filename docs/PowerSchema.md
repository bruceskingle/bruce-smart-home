# A power schema

A draft, November 2026. One way of describing a site's electricity, the same whoever supplies it and whatever measures
it, so that the Electricity page can move out of the Octopus module into the app and show GloBird's customers, and
anyone's, as it shows Octopus's.

## Why

The Electricity page, `src/octopus/electricity.rs` and `src/octopus/pages/electricity.rs`, is the app's best view of
what a household used, and it is Octopus's: it combines the Octopus meters' half hours, the Home Mini's readings and the
Tesla gateway's registers, with Octopus's rate labels, e.g. Home Low Rate. A GloBird customer has the same kinds of
things, a smart meter, a time-of-use tariff, solar, Powerwalls and a zappi, measured differently. Each source should be
turned into one shape, which one page draws.

## The sources

| Source | Intervals | What it gives | Times | Lag |
| --- | --- | --- | --- | --- |
| Octopus meters | 30 minutes | import split by rate label, e.g. Home Low Rate, EV Standard Rate; export | UTC | 1 to 2 days |
| Octopus Home Mini | about 10 seconds | import and export energy and power, not split | UTC | none |
| GloBird portal | 5 minutes | import (register `E1`), export (`B1`), each with the tariff's times, e.g. Peak 15:00 to 21:00 | NEM time, UTC+10 all year | about a day |
| GloBird portal costs | a day | cost by category: usage, supply, solar, credits | NEM days | about a day |
| Tesla gateway, by a capture device | 10 seconds | registers: site import and export, solar, battery in and out, load | UTC | none |
| myenergi zappi | a minute, and live status | the car's energy | UTC | none |

## The shape

Times are unix seconds, so every source's times are converted where it is read, e.g. a GloBird day's first reading
starts at midnight UTC+10, whatever the clocks in Sydney say.

**An interval** is a span of time, from `start` to `end`, of any length, and what flowed in it, in kWh:

```rust
pub struct PowerInterval {
    pub start: i64,
    pub end: i64,
    /// Energy from the grid, by use and by tariff band, e.g. (Ev, "Low"), (House, "Peak").
    pub import: BTreeMap<(Use, Band), f64>,
    /// Energy to the grid, by where it came from and by tariff band, e.g. (Solar, "Feed in").
    pub export: BTreeMap<(Origin, Band), f64>,
    /// What the site's own devices measured, where they did: solar made, the batteries' charge and discharge, the car.
    pub measured: Option<Measured>,
    /// Which sources it is from, e.g. the meter and the gateway, and whether any of it is estimated.
    pub sources: Vec<SourceId>,
    pub estimated: bool,
}

pub enum Use { House, Ev, Battery, Unknown }
pub enum Origin { Solar, Battery, Unknown }
/// A tariff band, as the supplier names it, e.g. "Home Low Rate", "Peak", "Offpeak".
pub struct Band(pub String);
```

- **Import is split two ways**: by use, where the gateway or the charger says, otherwise `Unknown`, and by tariff band,
  which is what it is billed at. Today's labels, e.g. "EV Low Rate" and "Battery Low Rate", are pairs of the two.
- **Bands come from the supplier**: Octopus's rate labels, or GloBird's charge types with their times of day. Each
  interval's energy is in exactly one band, as each half hour is billed at one rate on Octopus, and each 5 minutes
  falls in one of GloBird's periods.
- **Intervals from different sources do not line up**, 10 seconds, 5 minutes, 30 minutes. The combiner, today's
  `combine`, picks the finest measured source for each display period, and splits a coarser one into it only where it
  has to, as it does now with the Home Mini and the meters.

**A cost** is separate, as suppliers give it at a different granularity from the energy:

```rust
pub struct CostItem {
    pub start: i64,
    pub end: i64,
    pub category: CostCategory,
    pub band: Option<Band>,
    /// In the supplier's currency, see Site; negative for a credit.
    pub amount: f64,
    /// The kWh it is for, if it is for energy.
    pub quantity: Option<f64>,
}

pub enum CostCategory { Usage, Supply, Export, Credit(String), Other(String) }
```

- Octopus: a half hour's usage is its kWh at the band's unit rate, and the standing charge is a day's supply.
- GloBird: a day's usage, supply and solar, and credits such as the ZEROHERO credit and the Super Export top up. A
  day's usage cost is not split by band, but the bands' rates can be worked out from many days at once, each day being
  one equation of its bands' kWh, see the open questions.

**A site** is what a profile describes: its timezone, e.g. Europe/London or Australia/Sydney, for days and the axis;
its currency, GBP or AUD; its supplier's module; and its devices.

## Where the code goes

1. `src/power.rs`, the shape, and a `PowerSource` trait which a module implements to give a site's intervals and
   costs for a range.
2. The combiner, today's `combine` in `src/octopus/electricity.rs`, split: the part which picks and splits sources
   moves to `src/power.rs`; the Octopus-only parts, the Home Mini and the Intelligent Octopus estimate, stay in the
   Octopus module, as what its source gives.
3. The Electricity page and `stacked_bar_graph.rs` move to `src/views/` and `src/components/`, and draw intervals and
   costs, whoever gave them. The Octopus pages which show what Octopus says, Meters and Home Mini, stay where they are,
   as their point is to show Octopus's view, e.g. its own split of the car's import, rather than the best estimate of
   what happened.
4. `src/globird.rs` and `src/globird/`, the module: the portal client, its caches, and its `PowerSource`.

## Storage

For now GloBird's replies are kept as the other modules' are, in DataSets, a day a record. Unlike Octopus's, its
readings are not kept for ever: 35 days were there when asked on 4 November 2026, and another integration's author
found 21. So they must be fetched at least every few weeks, and are kept like captured data, never thrown away as a
cache would. In bruce-db they are a kind each, e.g. `globird.usage` with the NMI's hash in the hash key and the day
as the sort key, see bruce-db's design.

## Plan, to 12 November

1. The GloBird module: the client, login as the portal's web page does, with no retries and a stop at a captcha, the
   session kept between runs, and the replies cached by day. Tests against `testdata/globird`.
2. `src/power.rs`, and the Octopus Electricity page moved onto it, looking exactly as it does now.
3. GloBird as a `PowerSource`: a GloBird profile's Electricity page shows import by band, export, and each day's
   cost.
4. A profile for the GloBird account on this computer, to show its owner.

Tesla and the zappi for that site wait for a capture device there.

## Open questions

- [x] Are GloBird's tariff times, e.g. Peak 15:00 to 21:00, in NEM time or in Sydney's local time, which is an hour
  ahead in summer? NEM time: on each of the 35 days, the readings from 15:00 to 21:00 NEM add up to the peak `usage`
  exactly, so in summer the peak is 16:00 to 22:00 by Sydney's clocks.
- [ ] Export (`B1`) is mostly non-zero through the night on 2 November. The Powerwalls exporting, e.g. for the Super
  Export top up, or something else?
- [ ] The bands' rates, from many days of usage and cost: worth it, or are the rates on Jeff's bill enough?
- [ ] How far back does the portal go? It gave all of the 35 days asked for.
