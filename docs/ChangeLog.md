# Change Log

What has changed in each release of Bruce's Smart Home, called Marco Sparko before 0.5.0, the newest first: the new features, the improvements to existing
ones, and the problems which have been fixed. The releases themselves are on
[GitHub](https://github.com/bruceskingle/bruce-smart-home/releases).

## Release 0.5.0

> **Marco Sparko is now Bruce's Smart Home.** It keeps its profiles and data in new places, `.bruce-smart-home` and
> `.bruce-smart-home-data` in your home directory, and does not read Marco Sparko's, so set up your profiles again, see
> [Profiles](gettingStarted/profiles.md). Once you no longer need Marco Sparko's files, `.marco-sparko` and
> `.marco-sparko-cache`, you can delete them, see [Your Data](gettingStarted/cachedData.md#before-version-050).

### New
- **More than Octopus.** Each source of data is a module, and a profile can use several:
  - **myenergi**: a zappi's records of what it charged the car with, and of its grid clamp, fetched from myenergi.
  - **Tesla**: the Powerwall gateway's readings of the grid, the Powerwalls, the solar panels and the house, read from
    the gateway on your home network. The app only reads it, so it works alongside e.g. Home Assistant.
  - **GloBird**: a GloBird Energy customer's smart meter readings every 5 minutes, with each day's costs, and the
    bills, from GloBird's customer portal.
- **Electricity**, Home's first page, puts the modules together: your supplier's meter readings, which are what you
  are billed for, the Home Mini's readings every 10 seconds, and what the Tesla gateway and the zappi measured, which
  split the import into what the house, the car and the Powerwalls used. Each use is at the rates it was billed at,
  and the kWh and the cost at each rate agree with your bill. Until the meter's readings arrive, a few days later, the
  rate is estimated from your tariff and when the car was charging, and shown as estimated. Its **Flows** view shows
  where the energy went in each period.
- **Site**, a diagram of where in your house each module measures, from the grid connection through the circuits each
  one feeds, its **topology**, which the Electricity page needs to split the import correctly, as it differs from house
  to house, e.g. whether the car's charger is inside the Tesla gateway's grid clamp. The page suggests one from the last
  week's measurements, and checks any you draw against them.
- **Live**, the site's diagram with what each circuit is taking or giving now, with dots moving the way the energy
  flows, faster the more power there is.
- **Capture devices.** Small, always-on ESP32 boards with a screen, e.g. the Waveshare ESP32-S3-RLCD-4.2 or the
  Guition JC3248W535C, which fetch the Tesla gateway's, the zappi's and Octopus's data every few minutes while your
  computer is asleep or the app is closed, keep it on their SD cards, and show the latest on their screens. The app
  reads what they took when it next runs. Adopt one on the **Devices** page, which also shows each device's state and
  settings, and can flash the capture firmware onto a board plugged in by USB, refusing firmware built for another
  board.
- **Temperature and humidity.** Capture devices with a sensor of their own keep its readings, which Home's Temperature
  and Humidity page shows as graphs, a line for each room.
- **Polling.** While the app is open it keeps the data which is only available for a while, such as the Tesla gateway's
  and the zappi's, and the charging Octopus planned and carried out, every 5 or 15 minutes. Home's Polling section shows
  what each module did last and when it polls next, with **Poll Now**.
- **Copy and rename profiles**, from the **…** beside each profile in the profile menu. An offline copy shows the data
  as it was when copied, without logging in or fetching anything, e.g. to try a change to its topology.

### Improved
- **The macOS app is signed and notarized** by Apple, so it opens like any other app downloaded from the internet,
  without the `xattr` command earlier versions needed, see [Installing on macOS](gettingStarted/osx/index.md).
- **Your logins are encrypted.** API keys, passwords and tokens are encrypted in the settings files, with a key kept in
  your computer's own store of secrets, so a copy of the files, e.g. in a backup, does not give them away. On macOS the
  Keychain asks for your password to let the app read the key, see
  [Your secrets and the Keychain](gettingStarted/cachedData.md#your-secrets-and-the-keychain).
- **Everything is kept in one store**, each profile's in a directory of its own, which the capture devices keep theirs
  in too, so what they take is copied to the app as it is.
- **Fewer requests to Octopus.** The meters' readings are fetched a month at a time, and a month already fetched is not
  fetched again for 8 hours, so moving around the graphs, e.g. zooming in, no longer fetches anything, and the Loading
  dialog only shows when something takes a while.
- **EV charging split.** The car's share of each half hour's import follows what the car was measured to take where it
  was measured, with Octopus's own split shown alongside to compare.

### Fixed
- A property with more than one electricity meter, e.g. an old one which no longer works beside its replacement, now
  shows the readings of the meter with the active agreement, rather than the first one Octopus lists.

## Release 0.4.4

> **Windows:** Marco Sparko for Windows now comes only as a `setup.exe`, there is no longer a `.msi` installer. If you
> installed an earlier version from the `.msi`, uninstall it first (Settings, Apps, Marco Sparko, Uninstall), so that
> you do not end up with two copies. Your profiles and downloaded data are kept.

### New
- **Drill down by double clicking.** On the Graph view, double click a bar to see it in more detail: a month shows its
  weeks, a week its days, and a day its individual half hour readings.
- **Zoom by dragging.** Click and drag across bars in the graph to show just those bars.
- **Right click menu on the graph.** Right click a bar for:
  - **Zoom Out**, back to what was shown before the last drill down or zoom.
  - **Previous** and **Next**, the same period before or after the one shown.
  - **Export this bar**, the readings in that bar as CSV or JSON.
  - **Reload this bar** and **Reload this month**, which fetch the data again from Octopus, in case it has been
    corrected since it was downloaded. While a reload runs the graph stays on screen, with a dialog showing its
    progress and a Cancel button.
- **Quick date ranges.** The date range dialog has buttons for This Week, 7 Days, This Month and 30 Days, and for the
  period before each of them: Last Week, Prev 7 Days, Last Month and Prev 30 Days.
- **Previous and next period buttons** either side of the date range, to step back or forward by the same period. The
  next button is greyed out when the period already reaches the present.
- **EV Charging.** A new EV Charging page lists your Octopus smart charging devices, with the make, model and battery
  size of the EV configured in Octopus and the charger's power. Each has a **Charging Sessions** page which shows:
  - A **table** of its sessions, with when each started and ended, how long it took, the energy added, the average
    power, the cost, the energy added as a percentage of the configured battery size, and any problems Octopus
    reported.
  - A **graph** of the sessions, a bar for each session (which runs across days when a session does), or totalled by
    day, week or month with each session in its own colour, and a **trend** graph. Hover over a session to see its
    details, and double click to show just that session.
  - A **Categories** menu to choose which sessions are shown: smart or public charging, and those which reached a
    full charge, did not charge, or failed with an error.
  - The sessions are saved, so only new ones are downloaded, and can be reloaded from the right click menu.
- **Home Mini.** If you have an Octopus Home Mini, its readings of your meter every 10 seconds can be shown. Open it
  from the Device ID of the Home Mini in the Smart Device Network on the Meters or Properties page.
  - The readings as a **table**, a **graph** or a **trend**, of **Energy** or **Power**, for each reading, or totalled by
    minute, 10 minutes, half hour, day, week or month.
  - **Import and export.** If you export electricity, e.g. from solar panels, the energy exported is shown below zero
    on the graph and in its own column in the table.
  - Drill down, zoom, the right click menu and export work as on the Meters graph.
  - Octopus only keeps the last 30 days of Home Mini readings, so everything downloaded is saved, and readings older
    than 30 days are still shown from what was saved. Readings are downloaded in as few requests as possible, and if
    Octopus's limit of 125 requests an hour is reached, the download waits until it can continue, with a countdown,
    rather than failing.
- **Smart devices.** The Meters page also shows each property's **Smart Device Network**, the meters and in-home
  devices such as the Home Mini, described in plain words, and your **Smart Flex Devices**, e.g. EV chargers, which
  link to their charging sessions. The Properties page shows each property's meters and its Smart Device Network.
- **Fetching data on a schedule.** Run the app with `--fetch` and it fetches the latest data for the last 30 days, as
  if every page was opened, without opening its window, then exits. Scheduled to run e.g. once a week, the app is always
  up to date when it is opened, and readings which Octopus only keeps for a while, such as the Home Mini's, are never
  lost. See [Fetching Data on a Schedule](ScheduledFetch.md) for how to set it up on macOS, Windows and Linux.
- **Electricity (first version).** A new Electricity page combines everything known about your electricity into one
  graph or table: import above zero, split into the rates it is billed at, e.g. Home Low Rate and EV Low Rate, and
  export below zero.
  - Each half hour comes from the Home Mini's readings every 10 seconds where it has all of them, which also show the
    detail within the half hour, otherwise from the meters' half hourly readings. The two agree to within 0.1%.
  - The rates come from the meter's readings, which arrive a few days later. Until then they are estimated, shown in
    paler colours: the low rate in your tariff's cheap times, and on Intelligent Octopus also when the car was charging
    outside them, otherwise the standard rate.
  - Energy or Power, for each reading up to a month at a time, with drill down, zoom, the right click menu, Reload and
    Export as on the other graphs. Energy is drawn so that each bar's area is its energy, as bars can be different
    lengths.
  - Octopus's split of the import between home and EV is shown as it gives it, with the car's share placed where it was
    charging. Octopus does not appear to measure how much the car took when, so within a half hour this is only an
    approximation, e.g. a car whose charging tapers off at the end is shown at a constant power. We intend to revisit
    this, e.g. with the charger's own readings, probably in a later release.
- **Help.** Every screen has help beside it, on the right of the window, which describes what is on that screen, with
  links to more detailed help and pictures of the buttons it mentions. It follows you as you move around the app, e.g.
  from the list of meters to a meter's readings. The help button at the top right shows or hides it, everywhere at
  once, and it can be made wider or narrower by dragging its left edge.
- **Documentation.** The documentation is now an [online book](https://bruceskingle.github.io/marco-sparko/), to read
  from start to finish or search, with the same help as in the app, and new pictures of the app throughout.
- **Bills open in their own window.** Click a bill in the list and it opens in a window of its own, beside the list, so
  the list stays where it was, and several bills can be compared side by side. Bill windows stay in front of the main
  window, and close when it does.
  - The **Window** menu lists the open windows, to bring any of them to the front, and can **Arrange Bill Windows**
    side by side across the screen, or **Close** any of them, or all of them.
- **The log.** The important things the app does, such as each request to Octopus, how much it returned and how long it
  took, and anything which went wrong, are shown in a panel along the bottom of the window, with errors in red and
  warnings in yellow. It includes what a scheduled fetch does, even while the app is open. It is also kept in a file,
  and the log button at the top right shows or hides it.
- **Settings.** The gear button at the top right opens the settings of the profile, at the moment how much of the log
  is kept: the last 24 hours (the default), each time the app is opened, or up to a size.
- **Price per kWh** of each EV charging session, in the Charging Sessions table.
- **Open bills.** A bill which Octopus has not issued yet says so, as it is not shown in the Octopus app yet, and may
  change before it is issued.

### Improved
- The Measurements page is now called **Meters**, and opens on the Graph view.
- The bills are listed newest first.
- The Marco Sparko logo is at the left of the bar along the top of the window.
- The window no longer flashes white, with a large logo, as the app starts.
- On Windows the menus are in the usual order, File, Edit and Window.
- Profile names may contain underscores as well as letters and digits.
- The labels along the bottom of the graphs are shown on a shaded band, centred on what they label. As many are shown
  as fit, so a wide window labels every bar, and when they do not all fit the roundest times are kept, e.g. every
  hour rather than every half hour. Graphs of several days of readings have a band for each day.
- Periods which would put too many bars on the graph for the dates shown, e.g. 10 second readings over a week, are
  greyed out, so that dragging to zoom stays quick. Click one to see why, and zoom in to use it. If the period chosen
  is too short for the dates, the next longer one is shown until you zoom back in.
- Clicking Cancel while data is downloading shows that it is cancelling.
- The button which collapses the menu on the left looks neater.
- When Octopus is busy or refuses a request for being one too many, the progress shows a short reason, e.g. "Too many
  requests", rather than the whole error.
- Highlighting when the mouse is over a button in a group now shows its whole border, and greyed out buttons are
  highlighted in the same faded way as the other greyed out buttons.
- Data which has already been downloaded is no longer downloaded again just because only part of a month was looked
  at.
- The browser style right click menu (with Reload and other options which could leave the app in a broken state) no
  longer appears.

### Fixed
- A bill which Octopus replaced before issuing it, e.g. when it closed the billing period early, stayed in the list
  after it no longer existed. It is now removed.
- Bills were sometimes downloaded again from the very first one each day, rather than only the latest ones.

## Release 0.4.3

### New
- **Measurements.** A new Measurements page shows the consumption and costs of each meter point:
  - As a **table** of readings, a **graph** of stacked bars, or a **trend** graph which compares years, with a line
    for each year.
  - In **cost** or **units**, for each half hour, for each period at the same rates, or totalled by day, week or month.
  - Broken down into the rates Octopus reports, e.g. Home and EV usage at the low and standard rates. The Categories
    menu chooses which are shown, their colours and line styles, and can show a total instead.
  - The trend graph can show 7 or 30 day moving averages, or the cumulative total for each year. Its total is shown by
    default.
  - For any date and time range, with progress shown while data is downloaded, and a Cancel button.
  - **Export** of what is shown as CSV or JSON.
- **Properties.** A new Properties page shows each property's electricity and gas meter points, meters and tariffs.
- **Profiles:**
  - Create a new profile from the profile menu.
  - Each profile other than the one in use has a **…** menu with **Size…**, which shows how much disk space its
    downloaded data uses, and **Delete…**, which deletes it. Its login tokens are deleted immediately and its
    downloaded data is moved to the Trash or Recycle Bin.
  - The profile and downloaded data can be kept in another directory, e.g. on another drive, instead of your home
    directory. See [Profiles](gettingStarted/profiles.md#moving-the-data-directory). The directory in use is shown on the Home page.
- The window opens at the size and position it had when it was last closed.

### Improved
- Bills downloaded since the last time they were viewed are highlighted.
- Graph axis labels show just the numbers, with the units (e.g. kwh/day) shown once above them.
- When Octopus rejects a request, e.g. when logging in, the message shown includes the reason it gave.
- If the saved login can no longer be used, the login page says why.
- Network errors and rate limits while downloading are retried automatically.

### Fixed
- Some Windows computers could not connect to Octopus at all, failing with "403 Forbidden". Marco Sparko now connects
  in the same way on every computer, whatever the version of Windows.
- Login details saved on Windows are now reliably readable only by you.
