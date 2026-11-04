# GloBird

The GloBird module fetches a GloBird Energy customer's electricity data from GloBird's customer portal. It is new:

- **Meters**, the meters GloBird lists for the account, and the registers of the smart meter's readings, **E1** what was imported from the grid and **B1** what was exported to it. Click a register to see its readings every 5 minutes, by the tariff's charge types, e.g. Peak Usage and Offpeak Usage, as a graph or a table, as on Octopus's page of [a meter's readings](../octopus/meter.md). Home's [Electricity](../electricity.md) page puts the import and export together. Times and days are Sydney's, by the clock. **Reload** fetches the latest readings from GloBird.
- **Days**, each day's energy from the grid by the tariff's charge types, e.g. Peak Usage and Offpeak Usage, the energy exported, and the day's costs by category, in AUD. **Fetch** gets the days not cached yet, and the last few days again, as the meter's readings arrive a day or so late.
- **Cache Files**, every file it has fetched, as GloBird sent it.

Days run from midnight to midnight in Australian Eastern Standard Time (AEST, UTC+10) all year, without daylight saving, as GloBird's days and tariff times do, following the electricity market. So in summer, when the clocks in New South Wales, Victoria, the ACT and Tasmania go forward, a day starts at 1am by the clock there, and a peak of 3pm to 9pm is 4pm to 10pm by the clock.

GloBird keeps only a few weeks of readings, so the app keeps every day it fetches. Fetch at least every few weeks, or the days in between are lost.

[Logging in](login.md) explains what it needs to fetch your data.
