# Home Mini

The readings of your meter every 10 seconds, from the Octopus Home Mini, or another consumer access device, plugged in near it. They show far more detail than the half hourly readings, e.g. the kettle or the car charging.

**Octopus only keeps them for 30 days.** The app saves those it has downloaded, so it can show older ones, and a [scheduled fetch](../../ScheduledFetch.md) saves them even when the app is not opened.

Show them as a **Table**, **Graph** or **Trend**, see [using the graphs](../graphs.md). In the graph:

- **Energy** shows the kWh used in each period, and **Power** the average kW, which is easier to compare between periods of different lengths.
- **10s**, **1min**, **10min** and **30min** to **Month** choose the length of each bar. A day of 10 second readings is the most that can be drawn.
- ![Reload](../icons/reload.svg) **Reload** downloads the readings shown again, as far as Octopus still has them.
- Export, what you sent to the grid, is shown below the line.

Octopus limits how many of these readings can be downloaded in an hour. If a download is held up by the limit, the app waits until it can continue, and shows how long that is.

These readings are what the meter measured, they are not used for billing, which uses [the meter's half hourly readings](meter.md). The [Electricity](../electricity.md) page puts the two together.
