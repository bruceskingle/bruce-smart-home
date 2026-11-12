# Notes for Coding Agents

Bruce's Smart Home is a Rust desktop app, built with [Dioxus](https://dioxuslabs.com), for seeing a household's energy: its
use, costs and bills from Octopus Energy, and what its Tesla Powerwalls and myenergi zappi did. People's guides are in
[docs/developers.md](docs/developers.md), which links to the notes on releases, help, redaction and screenshots.

## Layout

- `src/main.rs` and `src/lib.rs`: the command line, the shared context, and the `Module` trait which each source of
  data implements, registered in `ModuleRegistrations`.
- The modules: `src/octopus.rs` and `src/octopus/` (GraphQL, pages and graphs), `src/tesla.rs` and `src/tesla/`,
  `src/myenergi.rs` and `src/myenergi/`.
- `src/data_set/`: the file-backed caches the modules' data is kept in, each with its own rules for when to fetch again.
- `src/polling.rs` and `src/capture_device.rs`: polling the sources, and reading what the capture devices polled.
- `src/components/` and `src/views/`: shared Dioxus components, and the app's own pages.
- `src/profile.rs`, `src/cache.rs`, `src/data_home.rs`: profiles, and where their settings and caches are kept.
- `src/redact.rs`: redacted, offline copies of a profile, for the docs and for testing without an account.
- `graphql/octopus/`: the Octopus schema and the queries, from which `build.rs` generates the typed code, using the
  `bruce-graphql` crates in a checkout of [bruce-graphql](https://github.com/bruceskingle/bruce-graphql) next to this
  one, on the branch of the same name.
- `capture/`: firmware for the ESP32 capture devices, built separately, see [capture/README.md](capture/README.md).
  It logs with the `log` macros, never `println!`, which panics on a device with no computer reading its USB port,
  see the platform's CLAUDE.md in bruce-esp-idf.
- `docs/`: the user guide, an mdBook, whose `docs/help/` pages are also built into the app's help panel.

## Building and Running

- Run the app with the Dioxus CLI, `dx serve`, or `dx run`. `cargo run` does not work, as Dioxus bundles the assets.
  Give the app arguments with `--args`, e.g. `dx serve --args="--profile=test"`; the `=` is needed when the value
  starts with `-`.
- `cargo build` and `cargo test` are fine for compiling and the tests.
- The VS Code debug launch, `.vscode/launch.json`, runs the app with `-v` and writes its stdout to `target/verbose.log`,
  overwritten each run: the GraphQL requests and replies, errors and cache activity. Read it after a run the owner
  reports, rather than asking for the log. `eprintln!` output is not in it.
- The code is not formatted with rustfmt, so do not run `cargo fmt` or treat `cargo fmt --check` as a failure; match
  the formatting of the code around a change.
- Try changes with a redacted, offline profile, made with `--redact <profile> <new profile>`, see
  [docs/Redaction.md](docs/Redaction.md), or with `--home=<an empty directory>` to start afresh, rather than with a
  real account. Never put an account's details, API keys or passwords in a commit, an issue or a
  log you share.

## Capture devices

- Read a device's serial log with `python3 capture/log.py <port>`, which leaves a board with a USB serial chip, e.g.
  the CYD, running, but restarts an ESP32-S3 on its own USB port, e.g. the RLCD and the Guitron (reset reason Usb). `espflash monitor` restarts
  it, and with `--non-interactive --no-reset` leaves it halted in its bootloader until `espflash reset`. The tasks
  "Device: Flash" and "Device: Log" are in `.vscode/tasks.json`.
- Before a request which changes a device, e.g. its settings, a restart, a release or a factory reset, check its state
  in the same command, e.g. `GET /api/info` says it is adopted, and stop if it is not as assumed: a "negative test" of a
  released device once wiped its Wi-Fi settings.
- Flash only with bruce-embedded-flash's `bruce-embedded-flash` (the cargo runner and the "Device: Flash" tasks use it),
  never espflash directly: it refuses firmware built for another board than the one on the port, from the record each
  image carries, and asks the owner which board it is if the firmware on it has no record. Never pass `--confirm-board`
  on the owner's behalf.
- Before flashing, check the board on the port is the one meant, by its MAC address, which is the device's ID in the
  app, e.g. `espflash board-info --port <port>`, which writes nothing but restarts it: the port names say only where a
  board is plugged in, and several boards may be.
- Flash or restart the devices one at a time, waiting until each polls again, so that one is always capturing, and
  away from the Octopus poll times, :29, :30, :59 and :00 past the hour.
- A device without an SD card keeps its records in memory, which a restart loses, so before flashing or restarting
  one, check that the app has read it lately, e.g. its last "GET /api/capture/..." in the profile's log, which it does
  only while a profile which is not offline is active.

## Branches

Each release is developed on a `vX.Y.Z` branch, in both this repo and bruce-graphql, which is squash merged to `main`
when it is released, see [docs/Release.md](docs/Release.md). Base a pull request on the release branch being developed,
not on `main`.

## Conventions

- **GraphQL nullability.** Do not put `@ms_required_by_default` on new queries in `graphql/octopus/`. Let the
  generated types follow the schema, so a field the schema allows to be null is an `Option`, and show `None` sensibly,
  e.g. blank, rather than unwrapping it. Octopus does return nulls the older queries assumed it would not.
- **Tooltips.** Anything with a tooltip (`title`) has the dotted underline cue, and shows its tooltip at once when
  clicked (`components/tooltips.rs`, `assets/scripts/tooltips.js`). Clickable things are left out of both: keep the
  `:not(...)` list in `main.css` and `CLICKABLE` in `tooltips.js` in step. A heading which sorts its column when clicked
  is a `th.sortable`, with arrows, and has no tooltip.
- **Module pages are faithful.** A module's own pages show its source's data as it gave it: the shortest period is the
  readings themselves, e.g. GloBird's 5 minutes or an Octopus meter's half hours, and longer ones only add them up,
  shown as derived. Nothing is interpolated, estimated or blended with another module's data on them; that is done
  only on Home's pages, e.g. Electricity, which put the modules together.
- **Caches hold only what the source sent.** A cache of an API's or a device's data holds its replies or records as
  they came, never values worked out from them, so that what was given can always be told from what was worked out.
  Computed values, if worth caching at all, go in a cache of their own, read alongside. The Octopus meter readings'
  cache breaks this, see bruce-db's docs/design.md, and is to be fixed with the data store.
- **Changes to what is stored are profile upgrades.** A change to the format of the data a profile keeps, e.g. a
  kind's keys or records, or to the profile file, bumps `CURRENT_VERSION` in `src/profile.rs` and adds a step to
  `upgrade_profile` which converts a profile of the version before, leaving the old data as it was. The Home page asks
  the user whether to upgrade a profile older than the app, and runs it when they agree; `--upgrade PROFILE` runs it
  from the command line. 0.5.0 is the move into the store, see `src/convert.rs`.
- **Derived numbers.** A number shown as a source gave it, e.g. GloBird's cost of a day or Octopus's kWh, is in the
  default colour; one the app works out, e.g. a total, a count, a rate fitted to costs or a split, has the `derived`
  class (`--derived` in `main.css`, pink), e.g. `td { class: "number derived", ... }`. Check the cache to tell which a
  value is: a sum of a source's several rows is derived, its only row is not. Say so in the page's help.
- **Errors** are shown briefly with the detail as a tooltip, with `ErrorText` (`components/error_text.rs`). Format an
  error with `{error:#}`, its chain of causes, not `{error:?}`, which includes a stack trace.
- **Help.** A change to what a screen shows or does is made to its page in `docs/help/` too. A new page is added to
  `PAGES` in `src/help.rs`, see [docs/WritingHelp.md](docs/WritingHelp.md).
- **Comments** are doc comments in full sentences, saying what a thing is for and why, as the surrounding code's are.
- **Modules are independent.** No module reads another's cache, calls its code, or is named in shared code; each gives
  its data through the app's interfaces, e.g. `Module::electricity_source`, `measurement_source` and `channels`, and
  the app puts them together, `src/power/`. Pages such as Home's Electricity belong to the app and work with any
  subset of modules, e.g. Tesla alone.
- **The site's topology is never assumed.** Where each meter and CT clamp measures differs from house to house, so the
  split of the import by use comes from the profile's topology, `src/power/topology.rs`, set on Home's Site page, see
  [docs/PowerSchema.md](docs/PowerSchema.md). Say "topology", not wiring, and that a circuit feeds another, or is fed
  from it, not "under" it, as the diagram goes from left to right.
- **What the supplier says, and what happened.** A module's pages show what its source says, e.g. Octopus's Meters
  page, including its split of the car's import, which Octopus models, e.g. the same energy in each half hour of a
  charge. Home's pages are the app's best understanding from everything available, e.g. the measured split of the
  import into the house's, the car's and the batteries', so they never present a supplier's split by use as fact. But
  the supplier's bill is a fact: in every bar on Home's pages the kWh and the cost at each rate, e.g. peak and
  off-peak, agree with the supplier's, and only who used them is the app's. A supplier's rate may depend on what it
  took the energy to be for, e.g. Octopus billed the car's charging at peak, inside the cheap times, past a limit with
  the charge cap off, while the house's was cheap; so the kWh it billed as a use go to that use as far as it was
  measured to use them, and only the rest to other uses, at the rate they were billed at. This is done through the
  shared schema, `src/power/`, from the rates, uses and costs a module gives with its readings, not by naming a
  supplier. The Home Mini is a consumer access
  device reading the same meter Octopus bills from.
- **ChangeLog.** `docs/ChangeLog.md` has a `# Release X.Y.Z` section for each release, with New, Improved and Fixed
  lists, written for the app's users: no code, refactoring or release process. Add to the section of the version in
  `Cargo.toml` when asked. Leave out fixes to how credentials or logs were handled unless asked.

## Octopus's API

- `rateLimitInfo { fieldSpecificRateLimits(first: 100) { edges { node { field rate ttl isBlocked } } } }` lists each
  limited field's rate, whether the caller is blocked, and `ttl`, when the block or window ends, as a unix time. A
  limited request fails with GraphQL error KT-CT-1199, not HTTP 429.
- Measured on 25 October 2026: `smartMeterTelemetry` 125 an hour, a fixed hour from the first request;
  `PropertyType.measurements` 20 a minute; 500,000 points an hour.
- `smartMeterTelemetry` at TEN_SECONDS gives 12 and 13 hours, an empty list with no error for 18 hours, and KT-GB-4039
  for 24, so the app asks for at most 12 hours at a time, `MAX_REQUEST_SECONDS` in `src/octopus/telemetry.rs`.
