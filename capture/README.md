# Bruce's Smart Home Capture Device

Firmware for an ESP32 which polls what Bruce's Smart Home needs while the computer it runs on sleeps, built on
[bruce-esp-idf](../../bruce-esp-idf), which must be checked out next to this repo, with
[bruce-embedded-std](../../bruce-embedded-std) and bruce-embedded-firmware-record. It polls the Tesla gateway,
myenergi's zappi and Octopus's smart charging, and keeps what it gets on an SD card. The boards it runs on, and what a
board needs, an ESP32-S3 with PSRAM and an SD card, are in the user guide's [Capture Device
Hardware](../docs/CaptureHardware.md), and the SD cards which have been tried in the platform's
[Hardware](../../bruce-esp-idf/docs/Hardware.md#sd-cards) page. What each board's screen shows is in `src/main.rs`'s
`board` and `src/screen.rs`. The Waveshare ESP32-S3-RLCD-4.2's **Temperature and Humidity Log** feature (`climate.rs`)
logs its own sensor every 10 seconds, as periods of each value, at `/api/capture/climate/records`, which the app's Temp
& Humidity page shows, and its screen shows its 18650 battery's voltage and roughly how full it is (`battery.rs`).
`sdkconfig.defaults.esp32s3` adds the ESP32-S3 boards' flash and PSRAM to `sdkconfig.defaults`.

## Building and flashing

From this directory, with the ESP Rust toolchain installed (see bruce-esp-idf), and bruce-embedded-flash's
`bruce-embedded-flash`, which flashes only firmware built for the board on the port, refusing firmware built for
another, and asking which board it is if the firmware on it does not say (see its README; install it with
`cargo install --path ../../bruce-embedded-flash/bruce-embedded-flash`). Flash with it, never with espflash
directly:

- `cargo build --release` for the Waveshare ESP32-S3-LCD-1.47B, the default board, and `cargo run --release` flashes
  the one plugged in by USB, and shows its log.
- For another board: `cargo build --release --no-default-features --features espidf,<board>`, with `<board>` one of
  `wave-esp32s3-rlcd-42`, the Waveshare ESP32-S3-RLCD-4.2, `wave-esp32s3-lcd-147`, the Waveshare ESP32-S3-LCD-1.47B,
  or `guition-jc3248w535c`, then `bruce-embedded-flash flash --port /dev/cu.usbmodem...
  target/xtensa-esp32s3-espidf/release/bruce-smart-home-capture`. Each board build needs `espidf` as well as its
  board, as `--no-default-features` turns it off.
- `python3 log.py [port]` shows a device's log without restarting it (needs pyserial). Not `espflash monitor`, which
  restarts it, or with `--no-reset` leaves it stopped in its bootloader. Each board's port is `/dev/cu.usbmodem*`,
  the chip's own USB.

Only ESP32-S3 boards with PSRAM and an SD card are capture devices, as each holds a replica of the store, see
[Capture Device Hardware](../docs/CaptureHardware.md). The small, cheap boards without them, e.g. the CYD (ESP32) and
the Waveshare ESP32-C6-Touch-LCD-1.47, were capture devices until 23 November 2026; they are to be display-only
devices, which show what a capture device has, with firmware of their own, not yet written.

`./build-firmware.sh` builds every board's firmware, or `./build-firmware.sh <board>...` those named, and copies each,
as soon as it is built, as boards with the same chip build to the same path in `target/`, to the shared firmware
folder, as `bruce-smart-home/dev/bruce-smart-home-capture-<board>.elf`, see bruce-embedded-flash's
`bruce-embedded-firmware`, e.g. `~/Library/Application Support/bruce-embedded-std/firmware/` on macOS, where
the app's Devices page and bruce-embedded-flash's window find it. `./build-firmware.sh --path <board>` prints where a
board's goes.

In VS Code, Terminal > Run Task has **Device: Flash RLCD** and **Device: Flash Guitron**, which find the port. Each
flash task builds its board's firmware with `build-firmware.sh`, then flashes its own file.

## A virtual capture device

Without an ESP32, the same firmware runs on a computer, macOS, Windows or Linux, in the board simulator of
bruce-embedded-std, which shows the board's screen in a window, see its `docs/Simulator.md`. `./build-sim.sh` builds
it into `target-sim/bin`, once for each board it simulates, with the stable toolchain, and
`target-sim/bin/bruce-smart-home-capture-sim --board rlcd42` (or `guitron`) runs it:

- **The screen** is the board's, in a frame of the board: the RLCD's black and white clock and numbers, or the
  Guitron's colour graph. Click its buttons, KEY and BOOT, to press them, and drag on the Guitron's screen to swipe it.
- **Its address** is the computer's, with a port of its own, chosen the first time it starts and kept, e.g.
  `192.168.1.190:60496`, which its screen shows, for HTTP and HTTPS alike. Give the app that, port and all, on its
  Devices page, to adopt it as a real one.
- **Its settings and store** are kept in a folder of its own, under the computer's folder for programs' data, e.g.
  `~/Library/Application Support/Bruce's Embedded Simulator/bruce-smart-home-capture/rlcd42` on macOS; `--data
  <folder>` keeps them elsewhere, e.g. for a second device of the same board. It needs no Wi-Fi set up.
- **It runs while its window is open.** It polls only then, so leave it open, minimised if need be.
- `--screenshot <file> --after <seconds> [--headless]` saves a picture of the board, e.g. for the docs, then ends.

It has no sensors: its temperature, humidity and battery read nothing. Its speaker is silent for now.

## Setting it up

The first time it starts, it makes its own WiFi network: join it and the config page opens. Enter your WiFi details,
then in the TeslaCapture section:

- **tesla_host**: the gateway's IP address.
- **tesla_email** and **tesla_password**: the gateway's customer login, as in the app. A password the gateway
  rejects is removed, so it is not tried again. Repeated failures lock out the customer login, for Home Assistant
  too.
- **tesla_schedule**: optional, when to poll, a cron spec with seconds first, `*/10 * * * * *` (every 10 seconds) if
  empty. The gateway is on the local network, so polling it often costs nothing outside the house. The log shows the
  free heap after each poll.

The config page never shows the password, and it is not written to the log: leave it empty to keep the saved one.
The page has no login though, so anyone on the network can change the settings, including tesla_host, which would
send the password to another address. TODO: a login for the config page.

Then enter the device's IP address on the app's Powerwall page.

## What the app reads

`GET /capture/tesla/records?since=<RFC 3339 time>` returns `{"device": {...}, "records": [...]}`:

- `device`: its version, when and why it last started (`resetReason`, e.g. `Brownout`), how many records it keeps,
  and its free heap.
- `records`: those after `since`, or all of them, oldest first, each as the app's own Tesla snapshots, with `slot`,
  `requested`, `received`, `capturedBy` and the `responses` of `meters/aggregates`, `system_status/soe`,
  `system_status/grid_status` and `operation`, keeping only the fields the app uses.

The records are only kept in RAM, so they are lost when the device restarts.

The same records are at `https://<device>/api/capture/tesla/records`, on the device's HTTPS API, which also has
`/api/info`, `/api/config` and `/api/restart`, see bruce-esp-idf's CLAUDE.md. Its certificate is the device's
own, so use `curl -k` for now; the app will pin its fingerprint, from `/api/info`, when it adopts the device.
