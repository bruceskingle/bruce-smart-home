# Electricity

Home's first page, the profile's electricity, from its modules: its supplier's, [Octopus Energy](octopus/index.md), described here, or [GloBird](globird/index.md), whose smart meter's readings every 5 minutes are shown the same way, with the import split into the tariff's charge types, e.g. Peak Usage, and the export below zero. Where a [capture device](devices.md) read a [Tesla](tesla/index.md) gateway, what it measured splits the supplier's import by what it was used for. A profile with Tesla but no supplier shows the gateway's own import, the house's and the Powerwalls', and export, for the periods it was read all through.

Where your electricity went, putting together the meters' half hourly readings, which are what you are billed for, the [Home Mini's](octopus/home-mini.md) readings every 10 seconds, which show what happened within each half hour, and, where a [capture device](devices.md) was reading your Tesla gateway, what it measured.

- **Import** is split by the rate it was billed at, and by what it was used for: the house (**Home**), the car (**EV**) and, where it was measured, the Powerwalls charging from the grid (**Battery**).
- **Export** is what was sent to the grid, shown below the line.
- The **Source** column of the table says which readings each period came from.

The meters' readings arrive a few days later. Until they do, the rate is estimated from your tariff's cheap times and when the car was charging, and the Home Mini's readings are used.

On an Intelligent Octopus tariff, charging the car outside the cheap times is at the cheap rate too, and the estimates allow for that.

The times are the site's, as the line under the heading says, e.g. "Times in Australia/Sydney (AEDT)" for GloBird, as the tariff's times are, and each tooltip says the zone, as it changes when the clocks do.

Show it as a **Table** or **Graph**, see [using the graphs](graphs.md). In the graph, **Energy** shows kWh and **Power** the average kW. Short periods, down to 10 seconds, use the Home Mini's readings where there are all of a half hour's.

## Flows

Where the site's [topology](site.md) is set and its devices measured, e.g. by a [capture device](devices.md) reading a Tesla gateway, **Flows** shows two graphs instead: **Where it came from**, the grid, the solar panels and the Powerwalls discharging, and **Where it went**, each kind of load, e.g. the car (**EV**) and the house (**Home**), the Powerwalls charging, and export. Unlike the Graph, which only splits what came from the grid, these include what the solar panels and the Powerwalls gave the house, so they show what each load used. Each half hour is worked out from the topology, as the [checks](site.md#checks) are, and periods of a day or longer add them up. Periods down to a minute are shown, each worked out from the Home Mini's readings and the devices' over it, e.g. the tumble dryer's heater switching on and off, which the Powerwalls follow. Half hours the devices did not measure all through, e.g. before there was a capture device, have only the grid's import, as **Not measured**, and export. The shortest period is a minute, as the zappi's records are of minutes. Drag across the graphs to zoom in, and double click a bar to see it in more detail, as on the Graph.

## Measured or Octopus's split

Where the Tesla gateway was read all through a half hour, and the [Site](site.md) page says where it measures, what the import was used for is **measured**. E.g. where the zappi is outside the gateway's meter, the car's is the Home Mini's import less the gateway's, the Powerwalls' is what they took in less what the solar panels made, and the rest is the house's. Until the Site page is set, the import is split by rate only. Over a night's charging the car's measured import was within 0.2% of the zappi's own total. The Source column says **Tesla** for these.

Elsewhere, e.g. before a capture device was reading the gateway, the car's share is what Octopus says, from the meter's EV labels, drawn as a block at the charger's power within each half hour, which is an estimate.

Octopus's split of the car's import is not measured: it is the same in every half hour of a charge. To compare, where the import was measured and the meters have the half hour, the graph draws two lines over the bars, both a level for each half hour, as Octopus only splits whole half hours: **Octopus's EV split**, dashed, and **Measured EV, half hourly**, solid, what the car measured took in each half hour. Where they are apart, Octopus's split was out in that half hour. With the lines, the car's import is at the bottom of the bars, so that they line up with its top. Under the graph it says how much the car took, measured and as Octopus split it. Turn the lines off in the categories menu, as Octopus's EV split. The table and the export have Octopus's split as a column. The meters' readings arrive a few days later, so there is nothing to compare for the last few days.

The split only changes what you pay outside the tariff's cheap times. There, while Octopus is charging the car, its share is at the low rate and the rest at the standard rate, so each kWh Octopus counts as the car's beyond what it measured took is cheaper. Inside the cheap times every kWh is at the low rate, whoever used it. So under the graph it also says how much the car took outside the cheap times, measured and as Octopus split it, and, on a day and night tariff such as Intelligent Octopus Go, what the difference was worth.
