# For Developers

Bruce's Smart Home is open source, written in Rust with [Dioxus](https://dioxuslabs.com). The source code, issues and releases are on GitHub at [bruceskingle/bruce-smart-home](https://github.com/bruceskingle/bruce-smart-home).

The documentation for developers is kept with the source code, rather than in this book:

- [Writing Help](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/WritingHelp.md), how the help in the app and in this book is written and built in.
- [Release](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/Release.md), how a release is built and published.
- [Redaction](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/Redaction.md), how the account holder's details are replaced in the screenshots and examples.
- [Screenshots](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/Screenshots.md), how the screenshots of the app in the documentation are taken.
- [The Store](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/Store.md), how the app and the capture devices keep their data in bruce-db, the record store they share.
- [Data for agents](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/DataForAgents.md), the guide to the data for AI agents, which the app writes into the data directory as AGENTS.md, see src/agents_guide.rs: change it with any change to what is stored.
- [The Octopus Energy plugin](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/octopus/index.md), notes on Octopus Energy's GraphQL API, including the queries the Octopus web site makes.
- [GraphQL Playground HowTo](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/howto/GraphQL.md), an introduction to GraphQL and to making queries interactively.
- [Chrome Browser Inspector HowTo](https://github.com/bruceskingle/bruce-smart-home/blob/main/docs/howto/ChromeInspector.md), watching the API calls a web page makes.

This book is built from the `docs` directory with [mdBook](https://rust-lang.github.io/mdBook/), see `book.toml`, and published to GitHub Pages whenever `main` changes.

## Signing debug builds on macOS

The app keeps the key to its saved secrets in the Keychain, which lets a program read an item by its code signature.
Cargo's builds are signed ad hoc, a signature which is new with every build, so macOS asks to allow each new build to
read the key the first time it runs, and the window waits for the answer. To stop it, sign debug builds with a code
signing identity of your own, e.g. an Apple Development certificate from Xcode, in a git-ignored `.cargo/config.toml`:

```toml
[target.aarch64-apple-darwin]
linker = "scripts/link-and-sign.sh"

[env]
DEBUG_SIGNING_IDENTITY = "Apple Development: Your Name (TEAMID)"
```

`security find-identity -v -p codesigning` lists the identities you have. `scripts/link-and-sign.sh` links as usual,
then signs each executable with the identity, with its name, without cargo's hash, as its identifier, so that every
build has the same signature. Choose **Always Allow** when macOS asks the first time. Without the file, builds are
signed ad hoc, as before, and nothing else changes.
