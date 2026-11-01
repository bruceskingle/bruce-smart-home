# Notes for Coding Agents

Marco Sparko is a Rust desktop app, built with [Dioxus](https://dioxuslabs.com), for seeing a household's energy: its
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
  `sparko_graphql` crates in a checkout of [sparko_graphql](https://github.com/bruceskingle/sparko_graphql) next to this
  one, on the branch of the same name.
- `capture/`: firmware for the ESP32 capture devices, built separately, see [capture/README.md](capture/README.md).
  It logs with the `log` macros, never `println!`, which panics on a device with no computer reading its USB port,
  see the platform's CLAUDE.md in sparko-embedded-std.
- `docs/`: the user guide, an mdBook, whose `docs/help/` pages are also built into the app's help panel.

## Building and Running

- Run the app with the Dioxus CLI, `dx serve`, or `dx run`. `cargo run` does not work, as Dioxus bundles the assets.
  Give the app arguments with `--args`, e.g. `dx serve --args="--profile=test"`; the `=` is needed when the value
  starts with `-`.
- `cargo build` and `cargo test` are fine for compiling and the tests.
- The code is not formatted with rustfmt, so do not run `cargo fmt` or treat `cargo fmt --check` as a failure; match
  the formatting of the code around a change.
- Try changes with a redacted, offline profile, made with `--redact <profile> <new profile>`, see
  [docs/Redaction.md](docs/Redaction.md), or with `--home=<an empty directory>` to start afresh, rather than with a
  real account. Never put an account's details, API keys or passwords in a commit, an issue or a
  log you share.

## Branches

Each release is developed on a `vX.Y.Z` branch, in both this repo and sparko_graphql, which is squash merged to `main`
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
- **Errors** are shown briefly with the detail as a tooltip, with `ErrorText` (`components/error_text.rs`). Format an
  error with `{error:#}`, its chain of causes, not `{error:?}`, which includes a stack trace.
- **Help.** A change to what a screen shows or does is made to its page in `docs/help/` too. A new page is added to
  `PAGES` in `src/help.rs`, see [docs/WritingHelp.md](docs/WritingHelp.md).
- **Comments** are doc comments in full sentences, saying what a thing is for and why, as the surrounding code's are.
