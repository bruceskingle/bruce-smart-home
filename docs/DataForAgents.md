# Bruce's Smart Home's data, for AI agents

This is for an AI agent, e.g. Claude, asked to explain or analyse a household's energy data from
[Bruce's Smart Home](https://github.com/bruceskingle/bruce-smart-home). The app writes it, with a section about the
installation it is in, as `AGENTS.md` in its data directory, usually `~/.bruce-smart-home-data`, each time it starts.

**This is a household's personal data:** its use of energy, when it is home, its bills and its devices. Use it only for
what the person asking wants, and do not send it anywhere they have not asked for.

**Never read the settings directory**, usually `~/.bruce-smart-home`, nor the files `~/.bruce-smart-home-dir`,
`~/.marco-sparko*`: they hold the household's passwords and API keys. Everything an analysis needs is in the data
directory, and the installation's section below gives what it needs from the settings, e.g. each domain's topology.

## What is where

```
~/.bruce-smart-home-data/
  AGENTS.md                this file, with the installation's domains at the end
  store/<site>/            each domain's data, a site of its own, see the installation's section
  logs/<domain>.log       what the app did, e.g. each request and poll, and any errors
  old-cache/               data from before 0.5.0, in older formats, which the app no longer reads
```

A domain is a household, or a copy of one: e.g. `Bruce` and `Jeff` are two households, and `Bruce_offline` a
redacted, offline copy of `Bruce`. Each domain uses some of the app's modules, each a source of data: **octopus**
(Octopus Energy, a UK supplier), **globird** (GloBird Energy, an Australian supplier), **tesla** (a Tesla Powerwall
gateway, read on the home network), and **myenergi** (a zappi car charger and its harvis).

## The store

Each site is a [bruce-db](https://github.com/bruceskingle/bruce-db) store, of JSON lines:

```
store/<site>/
  journal/2026-11-17.jsonl            every record written that day, in the order written
  <kind>/<id>/2026/11/17.jsonl        a series, e.g. readings: a file a day, by sort key
  <kind>/<id>/records.jsonl           a DataSet whose sort keys are not dates
  blobs/<ab>/<64 hex characters>      files, e.g. a bill's PDF
  state.json                          the store's own bookkeeping
```

Each line is a record:

```json
{"n": 1580, "t": "2026-11-06T13:00:19.896681Z", "o": "app-de2ead544ab6",
 "hk": "<site>/globird.usage/<id>", "sk": "2026-11-03#B1#Standard Solar Feed in",
 "v": "2026-11-06T13:00:19.896557Z", "a": { ... }}
```

- `hk`, the **hash key**, names the DataSet: the site, the **kind**, e.g. `globird.usage`, and the rest, e.g. the
  account or the device. IDs which identify the household, e.g. account numbers, meter serials, NMIs and MPANs, gateway
  and zappi serials, are hashed, and cannot be turned back; a capture device's ID, its MAC address, is not.
- `sk`, the **sort key**, orders the records of a DataSet, usually a time, e.g. a reading's.
- `a` is the record itself: **what the source sent, as it sent it**, e.g. an item of an API's reply. The app never
  changes a value, so the units and field names are the source's, see [Kinds](#kinds).
- `v` is the record's version. A record may be written again, e.g. when a day's readings are fetched again; **the one
  with the newest `v` for each `hk` and `sk` is current**. To read a DataSet, read its files and the journal, and keep
  the newest version of each record.
- `o` is what wrote it: `app-<12 hex>` is the app, and 12 hex digits a capture device, an ESP32 which polls the sources
  around the clock and whose records the app copies.
- Records whose sort key starts `#`, e.g. `#fetched/2026-11-03`, are the app's bookkeeping: what was asked for, and
  when.

## Kinds

The units, times and fields are the source's. The ones which are not obvious:

| Kind | What it holds |
| --- | --- |
| `octopus.measurementsconsumption` | The electricity meter's readings, each half hour, in kWh, with what Octopus billed each at: rates such as Low Rate and Standard Rate, and its split of them by use, e.g. EV, which Octopus models, rather than measures. The schema is `graphql/octopus/` in the repository. |
| `octopus.telemetry` | The Octopus Home Mini's readings of the same meter, every 10 seconds: the import's cumulative total, in Wh, and the power, demand, in W. |
| `octopus.flex` | Octopus's smart charging of the car: the dispatches it planned and carried out. |
| `octopus.tariff` | Each change of the meters' tariffs, with their rates, kept as they were seen, as Octopus only gives the current one. |
| `octopus.bills`, `octopus.statementtransactions`, `octopus.*agreement*`, `octopus.properties`, `octopus.viewer`, `octopus.smartdevices2` | The account: bills and their transactions, the meters' agreements, the property and the devices. |
| `globird.usage` | The smart meter's readings, every 5 minutes, by register: `E1` is imported from the grid and `B1` exported to it, each by the tariff's charge type, e.g. Peak Usage, Offpeak Usage, Standard Solar Feed in. **Times are in the National Electricity Market's time, AEST, UTC+10, all year, without daylight saving.** A three-phase meter which keeps each phase apart records import and export in the same 5 minutes, e.g. while batteries on one phase export and the other phases import. |
| `globird.cost` | Each day's costs, in AUD, by category: `USAGE` (import), `SOLAR` (export credits, negative) and `SUPPLY` (the daily charge). GloBird does not give its rates; the app works them out from these. |
| `globird.account`, `globird.meters` | The account and its meters. |
| `globird.mail`, `globird.mail.detail`, `globird.document` | The emails GloBird sent, e.g. bills; a bill's PDF is a blob. |
| `tesla.snapshot` | The Tesla gateway's meters, every few minutes: the site (the grid, as the gateway's clamp measures it), the batteries, the solar panels and the load (the house, as the gateway works it out), each with its power in W and its cumulative energy imported and exported, in Wh, and the batteries' charge. |
| `tesla.details` | The gateway's configuration, which rarely changes. |
| `myenergi.status` | Each poll of the myenergi hub: each device's status. A zappi's `pst` is the plug (A no car, B1 connected, C2 charging), `div` the power to the car, `grd` the grid's and `gen` generation's, in W, `che` the kWh added since the car was plugged in, and `ectp1`-`ectp6` each CT clamp's power, in W, with `ectt1`-`ectt6` what each measures, e.g. Grid, Generation, AC Battery, Internal Load; clamps 4 to 6 are on its harvis. A harvi's `ectp1`-`ectp3` and `ectt1`-`ectt3` are its own clamps. |
| `myenergi.minutes` | A zappi's record of each minute, by UTC day, **in joules in the minute**: `imp` imported and `exp` exported by its grid clamps, `gep` generated, `h1d`-`h3d` diverted to the car on each phase from surplus, `h1b`-`h3b` boosted, from the grid, and `pect1`-`pect3`, `nect1`-`nect3` each of the zappi's own clamps' import and export. Divide by 3,600,000 for kWh. myenergi keeps no records of a harvi. |
| `climate.period` | A capture device's temperature and humidity sensor's readings, as periods. |
| `device.info` | Each capture device: its board, firmware and settings. |
| `app.*`, `device.converted` | The app's bookkeeping, e.g. `app.poll`, a log of each poll. |

## What is not in the store

The store has only what the sources sent. The app works the rest out each time it shows it, so **an analysis has to do
the same, or ask the person to read it from the app's pages**:

- **The split of the import by use**, e.g. the car's, the house's and the batteries', on Home's Electricity page. It
  comes from the site's **topology**, which says where each module measures, e.g. that the zappi is behind the Tesla
  gateway's grid clamp, see the installation's section. A circuit with no channel has what is left of what feeds it.
  The supplier's bill is a fact: in each period the kWh and the cost at each rate are the supplier's, and only who used
  them is worked out.
- **GloBird's rates**, worked out from each day's costs and kWh, on its Rates page.
- **The flows**, where energy came from and went, including the solar panels' and the Powerwalls' to the house.

The app's help, in `docs/help/` of the repository, describes each page and how its figures are worked out, and
`docs/PowerSchema.md` and `docs/Store.md` the design.
