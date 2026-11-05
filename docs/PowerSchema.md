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
| Octopus Home Mini | about 10 seconds | the same meter as Octopus bills from, read by the Home Mini, a consumer access device (CAD) on the meter's home area network: import and export energy and power, not split | UTC | none |
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

### What was built first

The shape above is not built yet. What is, in `src/power/`, is the Electricity page's own, put together by the app from
whatever the profile's modules give, none of which knows of the others:

- Each module gives its `SiteData` for a range through an `ElectricitySource`: a supplier's meters' readings, by
  label, e.g. Octopus's half hours by rate or GloBird's 5 minutes by charge type, a grid meter's readings every few
  seconds, e.g. the Octopus Home Mini's, and hints for estimating the rate of import not billed yet, e.g. Octopus's
  cheap times. A device, e.g. the Tesla gateway, gives its site meter's readings as grid readings.
- A module may give `Measurements` of the site's use, e.g. the Tesla gateway's registers, which split the grid's
  import into the car's, the batteries' and the house's, see `power/measured.rs`.
- The app, `power::electricity`, takes the meters' readings of the first module which has any, the grid readings of
  the first which has any, suppliers first, and the measurements, and `power/combine.rs`, which was Octopus's,
  combines them, as it did, into periods with the import by label and the export.

Next: the zappi's grid CT and charging, from myenergi, as grid readings and measurements of the car, which would give
a GloBird site, e.g., a real Electricity page without a capture device; then the use and band split of the labels, and
the costs.

GloBird's page shows Sydney's time for now, so in summer its days start an hour before GloBird's AEST days, and a
day's totals differ from the Days page's. A site's timezone, and whether its days are the supplier's or the clock's,
is a setting to come.

## The site's topology

Where each meter measures differs from house to house, so it is a setting of the site, its **topology**, never
assumed. Installers usually fit the zappi's grid CT clamp and the Tesla gateway's grid clamp at the grid connection,
both measuring the same thing, with the car's charger behind the gateway's clamp, so the Powerwalls see the car as
the house's load and discharge into it. The first household had the gateway's clamp moved so that the car is outside
it: there the car's import is the grid's less the gateway's site meter's. In other houses there is no gateway, or the
clamps are somewhere else again. Until a site's topology is set, import is shown by rate only, which is true whatever
the topology.

Other devices to keep in mind: myenergi also makes the libbi, a home battery, the eddi, which diverts solar power that
would be exported into e.g. a hot water cylinder, and the harvi, a wireless CT sensor, which may be how a zappi's or
an eddi's grid or solar clamp is read. Each is more channels, of a battery, of a load, or of a grid or solar clamp.

### Channels

Each module says what it measures as **channels**, without knowing where they are: a series of energy, import and
export, each with an id the setting refers to, e.g.

| Module | Channel | What it is |
| --- | --- | --- |
| Octopus | `octopus:electricity` | the billing meter, half hourly, and, where there is a Home Mini, the same meter every 10 seconds, read by the Home Mini, a CAD on the meter |
| GloBird | `globird:electricity` | the billing meter, its import (`E1`) and export (`B1`) registers, every 5 minutes |
| myenergi | `myenergi:Z12345678`, `myenergi:Z12345678:grid` | a zappi's own charging, and its grid CT clamp, every minute; later a libbi's, an eddi's and a harvi's |
| Tesla | `tesla:site`, `tesla:battery`, `tesla:solar`, `tesla:load` | the gateway's meters, every 10 seconds, read by a capture device |

A channel is one place in the topology, however many resolutions it comes in: the Octopus meter's half hours and its Home
Mini readings are one channel, the finer used where there are some, as now. Two channels on the same circuit, e.g. the
billing meter and a grid CT clamp at the same connection, are two measurements of one thing, to check one against the
other and to fill each other's gaps.

### The tree

The topology is a tree of circuits, from the grid connection down, each with what it is for and the channels which
measure it. The first household, with the zappi beside the gateway:

```
Grid                    octopus:electricity, myenergi:Z…:grid
├── Car                 myenergi:Z…
└── Gateway             tesla:site
    ├── Battery         tesla:battery
    ├── Solar           tesla:solar
    └── House           (nothing: the rest)
```

As installers fit it, with both grid clamps at the grid connection:

```
Grid                    octopus:electricity, myenergi:Z…:grid, tesla:site
├── Car                 myenergi:Z…
├── Battery             tesla:battery
├── Solar               tesla:solar
└── House               (nothing: the rest)
```

```rust
pub struct Circuit {
    pub name: String,
    /// What it is for, which labels its share of the import, e.g. EV Low Rate.
    pub use_: Use,              // Grid, Board, Car, Battery, Solar, House
    /// The channels which measure it, the most detailed first.
    pub channels: Vec<String>,
    pub children: Vec<Circuit>,
}
```

It is the app's setting of the profile, as the capture devices are, not any module's, as it is about how the modules'
channels relate to each other.

- A measured circuit's energy is its channels', the most detailed one which has the period.
- A circuit with no channel is its parent's less its measured siblings'. Only one child of a circuit can be unmeasured,
  or the split cannot be worked out, which the setting says when it is saved.
- Energy flows both ways, e.g. the battery charging and discharging, and the solar making. A circuit's share of its
  parent's import is its channel's import, a battery's less what solar panels on the same board made, as they charge
  it first, which is what the first household's split did; the measured circuits share what the unmeasured one does
  not take, in proportion. Within a half hour each use's share is placed by its power, smoothed, as before, see
  `power/topology.rs`.

### Checking it, and working it out

With some days of every channel fetched, the app can check a tree, and suggest one:

- **Checks.** Wherever a circuit and all its children are measured, the children's energy adds up to the circuit's,
  each half hour; two channels on one circuit agree; nothing derived is negative. The Site page shows, for each check,
  how far it is out, e.g. as a percentage of the energy over the days fetched, and the half hours worst out, so that a
  wrong tree, a clamp the wrong way round (its import and export swapped), or a missing load shows up.
- **Suggestions.** Some relationships are clear from the data. Whether the car is behind a meter: when the zappi
  charges at 7 kW, does that meter's import go up by 7 kW too? Whether two channels are one circuit: do they agree?
  Whether a clamp is reversed: does it agree with its import and export swapped? The app tries each placement of each
  channel that the checks allow, and suggests those which fit, which the user confirms, as only they know the house.
- The Tesla gateway's own load is itself the site, solar and battery's net, so it does not check anything; it is
  another way to the house's share.

### Drawing it

The Site page draws the topology as a diagram, the grid on the left and what it feeds to the right, each circuit in
its use's colour with its channels. The same picture is the basis of a live display, as the first household's NSPanel
shows from Home Assistant: the grid, the house, the solar panels, the batteries and the car, with each one's power
now and dots moving along the lines in the direction the energy flows, e.g. on Home or on a capture device's screen.
Unlike the NSPanel's, which is drawn for one house, it would be drawn from the site's topology.

### What is built when

1. This write-up, reviewed. Done.
2. The setting, kept with the profile's settings, and a Site page under Home to make it, with a diagram; the first
   household's assumption in `power/measured.rs` replaced by the split from the topology, `power/topology.rs`. Done:
   until a profile's topology is saved, its import is shown by rate only, so the first household saves its tree once.
3. The myenergi module's channels, the zappi's charging and grid CT, from its minute records, which myenergi keeps.
   Done, `myenergi/electricity.rs`. myenergi's records miss about 70 minutes a day, mostly one at a time; gaps of up
   to 2 minutes are filled with the average of the minutes either side, which makes 94% of the first household's
   half hours since October usable, against a third without. The suggested topology now comes from what each channel
   usually measures, e.g. a zappi's charging the car, so the topology code names no module.
4. The checks, over the cached days, then the suggestions. Done, `power/check.rs`, on the Site page, over the last 7
   days, of the topology as it is edited. Each channel is its net import each half hour, and the checks are
   conservation: what a measured circuit feeds adds up to it, two channels on one circuit agree, and the rest is never
   less than none. The suggestions are what the data says whatever the topology: channels which are the same, or one
   the other reversed, and channels which are the sum of two or three others, to within 5%. On the first household's
   data the zappi's grid clamp is the same as `tesla:site` to 0.8%, and the gateway's site is its battery, load and
   solar together, to 0.8%. Then a verdict, whether the topology looks right, each check within 10% and the rest less
   than none in no more than 2% of its half hours, and if not, a topology from the relations, which is offered if it
   fits: the same channels one circuit, a sum's circuit feeding its parts', each fed from the smallest sum it is in, a
   house channel in a sum the rest, unplaced channels fed from the grid, with the rest where the circuits fed do not
   add up, and the current topology's names kept.
4a. Categories, done: a circuit is one of the kinds which energy goes through differently, the grid, a board, a
   battery and solar panels, or a load of a category, Car, House and Unknown built in, and others the user adds on the
   Site page, e.g. Hot water, saved with the profile. The split gives each category's share, labelled with its prefix,
   EV for the car and Home for the house, as Octopus's labels have them, or its name, e.g. Hot water Low Rate. Colours
   chosen, for categories on the Site page and for labels in the graphs' Categories menus, are saved with the profile,
   a category's being the default of its labels'.
5. The live flow display, from the topology and the latest readings. Done, `power/live.rs` and Home's Live page: each
   circuit's power over the latest minute every placed measurement has, with dots moving along the lines, read from the
   capture devices every 10 seconds while the page is shown, the zappi's latest minute where it lags, marked with its
   time. Streaming from the devices was considered and left: they poll every 10 seconds anyway, and their web server
   handles one request at a time.
6. Done, `power/flows.rs` and the Electricity page's Flows view: graphs of where the energy came from and where it went, beside the Electricity graph, which mixes the two and is
   hard to read, as the first household's Home Assistant energy dashboard shows them: sources, the grid, the solar
   panels and the batteries discharging, and sinks, the house, the car, the batteries charging and export, each
   stacked over time, from the topology's split.

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
- [ ] How far back does the portal go? It gave all of the 35 days asked for, and then all of the 40 the module asks
  for, so at least 40.
- [ ] The ZEROHERO Credit and Super Export top up costs were there for 4 November while it was the current day, and gone
  two hours after it ended. Are they estimates which GloBird replaces, and where do they end up?
