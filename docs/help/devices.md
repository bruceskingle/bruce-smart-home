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
3. Type the code and click **Adopt**. The app turns on the device's **Store**, which keeps what it takes on its SD
   card, for this profile, and restarts it. Once it is back, the app reads what the device has taken that it does not
   have yet, see Polling in Home.

A device adopted before devices had a store, whose **Store** is off, or keeps no site, keeps nothing the app can read.
The first time the app finds one so, it turns its store on for this profile and restarts it, which loses nothing, and
its status says so. A device whose store keeps another profile's site is left as it is, and its status says so: adopt
it again in this profile to move it here.

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

## Flashing a board

**Flash a Board**, at the bottom of the page, puts the capture firmware on a board plugged into this computer by USB,
e.g. a new one, or updates an adopted one's. Click **Show the Boards on USB** for the list of USB serial ports, with
what the computer says of each. A board on its chip's own USB, e.g. an ESP32-S3 or -C6, shows its ID, its Wi-Fi MAC
address, and is named as adopted if it is one of this profile's devices, without touching it; a board with a USB
serial chip, e.g. a CYD, does not say. Each board identified on this computer is remembered, so the list says what it
was when last identified, for sure for a board which shows its ID, and only as **Maybe** for one on a USB serial chip,
as the computer names its port after the USB socket it is plugged into, and another board may be in it now.

A board plugged in while the list is shown is identified at once: the app reads which chip it is and which board its
firmware was built for. **Identify** does the same for any other, and **Restart** restarts it, e.g. to see which board
it is by its lights or screen. Both restart the board, so a device that is capturing loses a few seconds.

Click a board's row for what it is. If its firmware says which board it is, the app says so, and offers **Flash**,
with the newest of the firmware it runs, or for a device this profile has adopted, or a new board, the capture
firmware, and **Other Firmware**, for the other firmware for its chip, e.g. an older version. If its firmware does not
say, e.g. as it is new, or was last flashed before firmware said, pick which board it is from those with its chip,
each with what it looks like and its maker's page.

Flash asks for one more click, **Yes, Flash It**, if the firmware is for another board than its firmware says, as
firmware for the wrong board can damage it, or its firmware does not say which board it is, when **Restart It** lets
you see which board it is by its lights or screen, or the firmware would replace other firmware, e.g. a board's demo
over the capture firmware, or anything but the capture firmware on a device this profile has adopted. The app checks
again, just before flashing, that it is the same board and the right chip, shows how far it has got in a box over
the page, and restarts the board when it is done; the box then says how it ended until you **Close** it. A part of the firmware already on the board is not written again, so flashing the same
firmware again takes only a few seconds.

If the app cannot reach a board's bootloader over USB, start it by hand, as the page says for each board, e.g. hold
BOOT, press and release RESET, then release BOOT, and click **Identify** again.

The app offers the firmware on this computer, in a folder shared with the other apps which flash these boards, e.g.
`~/Library/Application Support/bruce-embedded-std/firmware/` on macOS, by where it came from and its version, e.g.
`bruce-smart-home/0.5.0`, the capture firmware, or `bruce-embedded-std`, the boards' demos. Every version is kept, so
**Other Firmware** can put an older one back.

## Getting firmware

**Get Firmware**, under the list of boards, finds the firmware on GitHub: the repositories with the topic
`bruce-embedded-firmware`, e.g. the capture firmware's, `bruce-smart-home`, and the boards' demos, `bruce-embedded-std`,
each with what its firmware is. **Releases** lists a repository's releases, and **Download** puts a release's firmware
on this computer, in the folder the list of boards flashes from. Tick **Keep up to date** to download a repository's
newest release each time Get Firmware is shown. **Add a repository** adds one without the topic, by its owner and name,
e.g. `someone/their-firmware`, if it has a `bruce-embedded-std.md` saying what its firmware is.

