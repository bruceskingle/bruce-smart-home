# Tesla

The Tesla module fetches data about Tesla Powerwalls from the Powerwall's gateway, on your home network. It is new, and for now it only shows what it fetches, to see what the data is like:

- **Powerwall**, what the gateway said at the time of the latest snapshot: the power of the grid, battery, home and solar, and their energy totals. **Take a Snapshot** asks the gateway again.
- **Cache Files**, every snapshot, as the gateway sent it, and once a day, or when the gateway's software changes, its details which rarely change, such as the Powerwalls it has.

The gateway only says what is happening now, it keeps no history. Its energy totals only ever increase though, so the energy between two snapshots is the difference between them.

The app only reads from the gateway, it never changes any of its settings, so it can be used alongside other programs which use the gateway, such as Home Assistant.

The app only takes snapshots while it is running and the computer is awake. A [capture device](../devices.md), a small always-on ESP32 with a screen, can take them instead, every 5 minutes, and keep them on its SD card, or for the last 12 hours without one. Once one has been adopted on the **Devices** page in Home, the app reads it rather than the gateway: **Read the Capture Devices** fetches the samples taken since the latest snapshot and saves them as snapshots, over an encrypted connection, with the key only this profile has. A capture device's snapshots only have what the **Samples** page shows, not everything the gateway says, and it does not fetch the gateway's details.

[Logging in](login.md) explains what it needs to fetch your data.
