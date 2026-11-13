# Notes for Coding Agents

Bruce's Smart Home is a Rust desktop app, built with [Dioxus](https://dioxuslabs.com), for seeing a household's energy: its
use, costs and bills from Octopus Energy, and what its Tesla Powerwalls and myenergi zappi did. People's guides are in
[docs/developers.md](docs/developers.md), which links to the notes on releases, help, redaction and screenshots.

Read the note for a kind of work before starting it:

- [agents/CaptureDevices.md](agents/CaptureDevices.md): before building, flashing or restarting a capture
  device, sending it a request, or reading its log.
- [agents/EnergyAccounting.md](agents/EnergyAccounting.md): before changing Home's pages or `src/power/`: the
  site's topology, and how the supplier's bill and what was measured are put together.
- [agents/OctopusApi.md](agents/OctopusApi.md): before adding or changing an Octopus query: its rate limits.

## Layout

- `src/main.rs` and `src/lib.rs`: the command line, the shared context, and the `Module` trait which each source of
  data implements, registered in `ModuleRegistrations`.
- The modules: `src/octopus.rs` and `src/octopus/` (GraphQL, pages and graphs), `src/tesla.rs` and `src/tesla/`,
  `src/myenergi.rs` and `src/myenergi/`.
- `src/data_set/`: the file-backed caches the modules' data is kept in, each with its own rules for when to fetch again.
- `src/polling.rs` and `src/capture_device.rs`: polling the sources, and reading what the capture devices polled.
- `src/components/` and `src/views/`: shared Dioxus components, and the app's own pages.
- `src/profile.rs`, `src/cache.rs`, `src/data_home.rs`: profiles, and where their settings and caches are kept.
- `src/store.rs`, `src/device_kinds.rs`: the store, bruce-db, which the app and the capture devices keep their data in,
  see [docs/Store.md](docs/Store.md) for how they use it, and bruce-db's docs/design.md for the store itself.
- `src/redact.rs`: redacted, offline copies of a profile, for the docs and for testing without an account.
- `graphql/octopus/`: the Octopus schema and the queries, from which `build.rs` generates the typed code, using the
  `bruce-graphql` crates in a checkout of [bruce-graphql](https://github.com/bruceskingle/bruce-graphql) next to this
  one, on the branch of the same name.
- `capture/`: firmware for the ESP32 capture devices, built separately, see [capture/README.md](capture/README.md)
  and [agents/CaptureDevices.md](agents/CaptureDevices.md).
- `docs/`: the user guide, an mdBook, whose `docs/help/` pages are also built into the app's help panel.

## Building and Running

- Run the app with `dx serve` or `dx run`, not `cargo run`, as Dioxus bundles the assets. Give it arguments with
  `--args`, e.g. `dx serve --args="--profile=test"` (the `=` is needed when the value starts with `-`). `cargo build`
  and `cargo test` are fine.
- The VS Code debug launch writes the app's `-v` output to `target/verbose.log`, overwritten each run: GraphQL
  requests and replies, errors and cache activity, but not `eprintln!` output. Read it after a run the owner reports,
  rather than asking for the log.
- Run `cargo fmt` after a change (`rustfmt.toml`, `max_width = 120`); in `capture/`, `cargo +stable fmt`.
- Try changes with a redacted, offline profile, `--redact <profile> <new profile>`, see
  [docs/Redaction.md](docs/Redaction.md), or with `--home=<an empty directory>`, not with a real account.

Each release is developed on a `vX.Y.Z` branch, here and in bruce-graphql, squash merged to `main` when released, see
[docs/Release.md](docs/Release.md). Base a pull request on the release branch, not on `main`.

## Conventions

- **GraphQL nullability.** Don't put `@bruce_required_by_default` on new queries: a field the schema allows to be null
  is an `Option`, shown sensibly, e.g. blank, never unwrapped. Octopus does return nulls.
- **Tooltips.** Anything with a tooltip (`title`) has the dotted underline cue and shows it at once when clicked
  (`components/tooltips.rs`, `assets/scripts/tooltips.js`). Clickable things are left out of both: keep the `:not(...)`
  list in `main.css` and `CLICKABLE` in `tooltips.js` in step. A heading which sorts its column is a `th.sortable`,
  with arrows and no tooltip.
- **Module pages are faithful.** A module's pages show its source's data as given: the shortest period is the readings
  themselves, longer ones only add them up, shown as derived. Nothing interpolated, estimated or blended with another
  module's data; that is done only on Home's pages.
- **Caches hold only what the source sent,** as it came; computed values, if cached at all, go in a cache of their
  own. The Octopus meter readings' cache breaks this, see [docs/Store.md](docs/Store.md).
- **Changes to what is stored are profile upgrades:** bump `CURRENT_VERSION` in `src/profile.rs` and add a step to
  `upgrade_profile` converting the version before, leaving the old data as it was. Home asks the user before
  upgrading; `--upgrade PROFILE` runs it from the command line. 0.5.0's is the move into the store, `src/convert.rs`.
- **Derived numbers.** A number as the source gave it is in the default colour; one the app works out (a total, a
  count, a fitted rate, a split) has the `derived` class (`--derived` in `main.css`, pink), e.g.
  `td { class: "number derived", ... }`. A sum of a source's several rows is derived, its only row is not; check the
  cache. Say so in the page's help.
- **Errors** are shown briefly with the detail as a tooltip, with `ErrorText` (`components/error_text.rs`), formatted
  with `{error:#}`, never `{error:?}`.
- **Help.** A change to what a screen shows or does is made to its page in `docs/help/` too; a new page is added to
  `PAGES` in `src/help.rs`, see [docs/WritingHelp.md](docs/WritingHelp.md).
- **Comments** are doc comments in full sentences, saying what a thing is for and why.
- **Modules are independent.** No module reads another's cache, calls its code, or is named in shared code; each gives
  its data through the app's interfaces, e.g. `Module::electricity_source`, and the app puts them together in
  `src/power/`. Home's pages work with any subset of modules.
- **Topology and the supplier's bill.** The site's topology is never assumed, and Home's pages agree with the
  supplier's bill in every bar; see [agents/EnergyAccounting.md](agents/EnergyAccounting.md).
- **ChangeLog.** Add to `docs/ChangeLog.md` only when asked, in the section of the version in `Cargo.toml`, under New,
  Improved or Fixed, written for users: no code, refactoring or release process, and no fixes to how credentials or
  logs were handled unless asked.
- Never put an account's details, API keys or passwords in a commit, an issue or a log you share.
