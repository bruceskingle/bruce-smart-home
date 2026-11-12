# Screenshots

The screenshots of the app in the documentation are taken by the app itself, from a redacted, offline, copy of a real
profile's data, see [Redaction](Redaction.md), so that they do not identify anyone and come out the same every time.

```
bruce-smart-home --redact <profile> demo
bruce-smart-home --home /Users/yourname --screenshots
```

or the **Redact Profile into Demo** and **Take Screenshots** launch configurations in VS Code. Run them in the
repository's directory.

The screenshots are taken in a data directory of their own, `/Users/yourname`, given with `--home`, so that they do not
show your own profiles. The shot list's `freshDataDirectory` clears it first, so that the first shots show what someone
sees the first time they run the app, e.g. its default profile and the help. Then a `loadProfile` step copies the demo
profile into the default one, for the rest. Create the directory once, owned by you.

Taking the pictures needs macOS's Screen Recording permission for what started the app, e.g. Visual Studio Code or
Terminal. The app asks macOS for it, which shows a prompt or adds it to the list in System Settings, Privacy &
Security, Screen & System Audio Recording, and stops if it has not been given. Turn it on there, adding the app with +
if it is not listed, then quit and restart that app, as macOS only applies the permission when it starts.

The pictures are blank if the display sleeps or the screen locks, so the app keeps the display awake until it exits,
with `caffeinate`, but do not lock the screen while they are taken.

The app opens with its window the size the shot list gives, goes through the shots, prints what it did, and exits.
Each picture is of the whole window, with its frame and shadow, as with ⌘⇧4 and Space, and replaces the file it is
saved to, so check them with `git diff` or by looking at the documentation, see `mdbook serve`.

## The Shot List

The shots are in `docs/screenshots.json`, or another file given after `--screenshots`.

```json
{
    "window": { "width": 1700, "height": 1024 },
    "shots": [
        {
            "file": "docs/readMe/Screenshot6.png",
            "steps": [ { "click": "octopus" }, { "click": "Meters" }, { "select": ".link" }, { "click": "Graph" } ]
        }
    ]
}
```

- `window` is the size of the window's content, in points. The documentation shows the pictures at most 1024 pixels
  wide, less in the book's column, so the bigger the window the more it shows, and the smaller its text. 1700 by 1024
  is the size the earlier screenshots were taken at.
- `freshDataDirectory` at the top is a data directory whose profiles and data are deleted before the shots, which must
  be the one given with `--home`, and not your usual one, or the app stops without deleting anything.
- `newProfiles` at the top lists profiles the shots create, e.g. `new_demo` to show a new profile's first run and the
  login page, which are deleted before the shots and afterwards, when the profile the app started with is made the
  active one again. Any profile with one of these names is deleted.
- `file` is where the picture is saved, relative to the repository.
- `help` is `true` to show the help panel, which is otherwise hidden. The log panel is always hidden.
- `pages` is `true` to show a module's list of pages on the left in the picture, which is otherwise hidden while the
  picture is taken, as it takes up space, for the shots about it. It is there while the steps are done, so they can
  click on it.
- `window` in a shot is `"newest"` to take the window opened last, e.g. a bill's, rather than the main window.
- `steps` are done in order, starting from the home page, each followed by a short pause for the app to catch up:
    - `{ "click": "Meters" }` clicks on what has that text, e.g. a button, a link or a page in the list on the left. It
      waits for it to appear, e.g. while a page loads.
    - `{ "select": ".bill-ref .link", "index": 1 }` clicks on the element a CSS selector selects, the first if `index`
      is not given, e.g. the second bill's reference.
    - `{ "row": "import", "select": ".link" }` clicks on the element the CSS selector selects in the first table row
      with that text, ignoring case, e.g. the import meter's link.
    - `{ "input": "input[type=date]", "index": 0, "value": "2025-01-01" }` types into a field.
    - `{ "scroll": "Consumption Analysis", "bottom": true }` scrolls the shot's window, e.g. a bill's, so that what
      has that text is at the top of it, or with `bottom`, at the bottom.
    - `{ "loadProfile": "demo" }` copies that profile from your usual data directory into the active profile, offline
      and without logins, and starts the app again with it.
    - `{ "wait": 2000 }` waits for that many milliseconds, e.g. for a window to open.
    - `{ "waitFor": "Categories" }` waits for that text to appear, e.g. once data has loaded.

A shot whose step fails, e.g. as the app has changed, is reported and not taken, and the others still are.

The demo profile shows its data as of when it was last fetched, see `clock.rs`, so the pages open on its latest data,
e.g. this month is the month it was last fetched in.
