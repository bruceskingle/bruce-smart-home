# Home

Home has its pages listed down the left, which ![Hide the page list](icons/hide-pages.svg) at its top hides, and ![Show the page list](icons/show-pages.svg) shows again.

## Modules

Modules lists every module, and whether the active profile uses it. Each module connects to one supplier or device: [Octopus Energy](octopus/index.md), [myenergi](myenergi/index.md) and [Tesla](tesla/index.md). Click a module here, or its name in the bar at the top, to open it.

**Data Directory** is where the app keeps its data, the profiles and a copy of everything it has downloaded. **Build Info** is the version of the app, useful when reporting a problem.

## Polling

Some data is only available for a while, and nothing else keeps it, such as what the Tesla gateway and the zappi say is happening now, the zappi's charge added since the car was plugged in, and the charging Octopus has planned and carried out, which it keeps for a few days, with whether charging is capped to its cheap hours. While the app is open it polls them, every 5 minutes, or 15 for Octopus, which limits how often it is asked, so that their history is kept. Polling shows, for each module, what it is doing, when it last polled and what it found, and when it polls next. **Poll Now** polls straight away, and **Pause** stops polling the module until **Resume**. A module which fails 3 times in a row is paused, so that a device which rejects its login is not asked again and again. Every poll is in [the log](log.md) too.

**From** says where a module's data comes from: **This computer** if the app polls it itself, or the names of the [capture devices](devices.md) which poll it, which the app reads instead, and then never polls it itself, so that the gateway, for one, is not asked twice. Where a device polls a module, the app still fetches the history its makers keep themselves, such as the zappi's day by day.

**Capture Devices** shows, for each device the app has read since it started, which modules it takes, when it was last read, how many records that gave and how many the app did not have already, or why it could not be read, where the device keeps them (on its SD card, or only in its memory, how many of how many), when and why it last started, which is in red if it was a crash or a dip in its power, and its free memory, with the least since it started as a tooltip.

Without capture devices, polling stops while the app is closed or the computer is asleep. The gateway's and the zappi's totals carry on counting meanwhile, so the energy of that time is still counted, only not when within it it was used. The zappi's charge added is only kept until the car is next plugged in, so open the app before plugging in the car.

## Devices

Devices lists the [capture devices](devices.md) the profile has adopted, small always-on boards which fetch data while this computer sleeps, and adopts more.

The buttons at the top right:

- ![Help](icons/help.svg) shows or hides this help.
- ![Log](icons/log.svg) shows or hides [the log](log.md) along the bottom of the window.
- ![Profile](icons/profile.svg) and the profile's name is the profile menu, which changes between [profiles](profile-size.md), or creates a new one.
- ![Settings](icons/settings.svg) opens the [settings](settings.md).

See the [help contents](index.md) for everything else.
