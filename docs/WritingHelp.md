# Writing Help

The help panel in the app shows a page about the screen being shown. The pages are the Markdown files in
[help](help/index.md), which are also part of this documentation, so the same text is read in the app and on GitHub.

## How the App Gets the Text

The pages are built into the app: `src/help.rs` reads each file with `include_str!` when the app is compiled, and
nothing reads them while it runs, so the installed app does not need the `docs` folder.

- **Editing a page** needs no code change. Cargo notices the file has changed, and the next build has the new text.
  `dx serve` does not hot reload it, the app is rebuilt.
- **Adding a page** means adding its path, relative to `docs/help`, to `PAGES` in `src/help.rs`, otherwise the app does
  not include it, and links to it go to GitHub instead.

## Which Page a Screen Shows

- **The app's own screens**, Home, the domain page and Settings, are chosen from the route in `Navbar`
  (`src/views/navbar.rs`).
- **A module's screens** are chosen by the module's `Module::help_topic`, from the page and the rest of its path, e.g.
  for Octopus in `src/octopus.rs`. A new screen, or a new path within a page, needs a line there. Until a module is
  logged in its page is `<module>/login.md`.
- **A bill's window** shows `octopus/bill.md`, set in `src/octopus/pages/bills.rs`.

A screen whose page does not exist says so in the panel, with the file it looked for.

## Links

Write links as ordinary relative Markdown links, so that they work on GitHub too.

- **Another help page**, e.g. `[using the graphs](../graphs.md)`, opens in the panel, with a back button to return.
- **A document in the book**, i.e. listed in `docs/SUMMARY.md`, e.g. `[scheduled fetch](../ScheduledFetch.md)`, opens
  in the browser on the published documentation, which is built from the `main` branch.
- **Any other document in the repository** opens in the browser on GitHub, also from the `main` branch.
- **A web site** opens in the browser.

A test, `links_between_help_pages_are_not_broken` in `src/help.rs`, checks that every link goes to a file which exists.
Run `cargo test` after changing links.

## Button Icons

Show a button by its icon, written as an image, e.g. `![Help](icons/help.svg)`, or `![Help](../icons/help.svg)` from
a page in `octopus`. The app draws it inline like a button, with the alt text as its tooltip, and GitHub shows the
image.

The icons are in [help/icons](help/icons/README.md), copied from the Lucide icons the app's buttons use, and listed
in `ICONS` in `src/help.rs`, which a new one must be added to. To add one, copy its elements from the `IconShape`
implementation of the icon in the `dioxus-free-icons` crate, `src/icons/ld_icons.rs`, into an SVG like the others.

## The Book

The documentation is also published as a book, with mdBook, see `book.toml` and `.github/workflows/docs.yml`, and the
help pages are part of it. A new help page should be added to `docs/SUMMARY.md` as well as to `PAGES`, so that it is in
the book. Preview the book with `mdbook serve --open`, after installing mdBook with `cargo binstall mdbook`.

## What to Write

Describe what is on the screen and what its controls do, for someone using the app rather than developing it. Keep
the screen's page short, and link to another page for more detail, e.g. the shared pages on
[choosing dates](help/date-range.md) and [using the graphs](help/graphs.md), rather than repeating them.
