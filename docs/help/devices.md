# Capture Devices

Capture devices are small always-on ESP32 boards with a screen, such as a CYD, which fetch data while this computer
sleeps, e.g. the Powerwall's and the zappi's every 5 minutes, and show the latest on their screens. The app reads what they fetched when
it next runs. **Devices** in Home's page list shows them.

A capture device needs a **micro SD card**, formatted FAT32, which is where it keeps what it fetches, a file a day,
until the app reads it. Without one it only keeps the last few of each in its memory, which are lost if it restarts. The Powerwall's readings every 10 seconds
take about 9 MB a day, so an 8 GB card holds over two years.

## Adopting a device

A new device has its own settings page in a browser, at its address followed by /config, which is where it is put on
your Wi-Fi. Once it is on the Wi-Fi, adopt it here:

1. Enter its IP address under **Add a Capture Device**, and click **Find**. The app shows what the device is.
2. Click **Show Pairing Code**. The device shows a 6 digit code on its screen for 2 minutes.
3. Type the code and click **Adopt**. The app then reads what the device has taken that it does not have yet, as
   its modules poll straight away, see Polling in Home.

The screen also shows the start of the device's certificate, which should be the same as the **Certificate** the app
shows. If it is not, something else on the network may be pretending to be the device, so do not adopt it.

Once adopted, only this profile can change the device's settings: its own /config page only says that an app manages
it, unless it cannot join the Wi-Fi, when it makes its own Wi-Fi network for its settings page again, with only the
Wi-Fi settings on it.

## Managing a device

The page lists the devices in a table, a row each, with its name, board, address, whether it is up and for how long,
why it last started, in red after a crash, and its restarts in the last 7 days, and checks every 30 seconds that each
can be reached. Click a device's ID, its Wi-Fi MAC address, for its card, below the table, with all of its details
and buttons; clicking another's shows its card instead.

- **Settings** shows each of the device's features, with its settings. Change them and click **Save**, then
  **Restart Now**, as they take effect when the device restarts. Passwords are never shown, only whether they are set:
  type a new one to replace it, or **Clear Password** to remove an optional one. **Fill in from this profile** fills in what the
  profile already knows for a feature, e.g. the Tesla gateway's login or the myenergi and Octopus API keys, to check
  before saving.
- The card shows what the device is, the board its firmware was built for and the **Chip** it is running on, with
  its flash and PSRAM, how long it has been running, and why it last started. **Restarts** counts how often it started in the last 7 days, by kind, crashes,
  watchdog resets and power dips in red, as they are trouble. Click the count for the list, with when each was. The
  device keeps its last 30 starts. Older firmware kept them on its SD card, so with no card they are
  **Not known**, and a card moved from another device brought that device's starts with it: update its firmware. If it
  last started after a crash, **Crash** says why, if the device knows: the error, or that it ran out of memory, with
  how much it wanted and how much was free.
  **Records kept on** says where it keeps its records: on its SD card, with how much of it is free, in red if less
  than a tenth is, or only in its memory, which loses them when it restarts, with why as a tooltip, e.g. that its SD
  card slot is empty. **Polls** lists the sources it takes, as it said when the app last read its records: any whose latest poll failed
  is in red, with the error, such as an API key the source did not accept, and when it last succeeded, as a tooltip.
  The log says when one starts failing, and when it polls again.
- A device with its own temperature sensor, e.g. the Waveshare ESP32-S3-RLCD-4.2, reads high, as the board warms it, by an amount set in its settings as **Temperature Offset**, in tenths of a degree. To set it, put a thermometer beside it for half an hour, type what the thermometer says into **Actual temperature now** under the offset, and click **Work out offset**, which fills in the offset from the device's latest temperature, then **Save**. It takes effect when the device restarts.
- A device with a battery, e.g. the Waveshare ESP32-S3-RLCD-4.2, shows its voltage over the last 2 days under its
  details, as it logs it with its temperature and humidity, so only while its **Temperature and Humidity Log** is on.
  The voltage says roughly how charged it is, but reads higher while it charges.
- **Refresh** asks the device again what it is, and **Restart** restarts it, when it can be reached.
- **Release** gives the device back to its own /config page, and removes it from this profile.
- **Forget** only removes it from this profile, e.g. if it no longer works. It stays adopted until its **BOOT** button
  is held down for 5 seconds, which releases it.

The app keeps a key for each device it adopts in the profile, which lets it change the device's settings, so keep the
profile's files private. A redacted copy of a profile does not include its devices.
