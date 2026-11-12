# Bruce's Smart Home Capture Device

Firmware for an ESP32 which polls what Bruce's Smart Home needs while the computer it runs on sleeps, built on
[bruce-esp-idf](../../bruce-esp-idf), which must be checked out next to this repo, with
[bruce-embedded-std](../../bruce-embedded-std) and bruce-embedded-firmware-record. It polls the Tesla
gateway, myenergi's zappi and Octopus's smart charging, and keeps what it gets on an SD card. It runs on:

- a CYD (ESP32-2432S028R), whose screen, on its side, shows a graph of the gateway's power, as the Waveshare's does, the
  default. It has too little memory for the internet polls, Octopus and myenergi, alongside the app's requests, so
  turn those off for it in the app's Devices > Settings, and it is mostly a display;
- a Waveshare ESP32-C6-Touch-LCD-1.47, an ESP32-C6, whose small screen shows only the numbers, with the
  `wave-esp32c6-touch-147` feature. Its SD card slot shares the display's SPI bus.
- a Waveshare ESP32-S3-RLCD-4.2, an ESP32-S3 with 8 MB of PSRAM, whose black and white reflective screen shows the
  clock with the latest numbers below, and the status for a while when its KEY button is pressed, and plays `sounds/Startup.wav` as it starts, with
  the `wave-esp32s3-rlcd-42` feature. Its **Temperature and Humidity Log** feature (`climate.rs`) logs its own sensor every 10
  seconds, as periods of each value, at `/api/capture/climate/records`, which the app's Temp & Humidity page shows. Below it the screen shows its 18650 battery's voltage and roughly how full it
  is (`battery.rs`), read every 10 seconds and in the log every minute. `sdkconfig.defaults.esp32s3` adds its flash and PSRAM to `sdkconfig.defaults`.

## Building and flashing

From this directory, with the ESP Rust toolchain installed (see bruce-esp-idf), and bruce-embedded-flash's
`bruce-embedded-flash`, which flashes only firmware built for the board on the port, refusing firmware built for
another, and asking which board it is if the firmware on it does not say (see its README; install it with
`cargo install --path ../../bruce-embedded-flash/bruce-embedded-flash`). Flash with it, never with espflash
directly:

- `cargo build --release` for a CYD, and `cargo run --release` flashes the CYD plugged in by USB, and shows its log.
- For the Waveshare, which is RISC-V: `MCU=esp32c6 cargo build --release --target riscv32imac-esp-espidf
  --no-default-features --features wave-esp32c6-touch-147`, then `bruce-embedded-flash flash --port
  /dev/cu.usbmodem... target/riscv32imac-esp-espidf/release/bruce-smart-home-capture`.
- For the RLCD: `MCU=esp32s3 cargo build --release --target xtensa-esp32s3-espidf --no-default-features --features
  wave-esp32s3-rlcd-42`, then `bruce-embedded-flash flash --port /dev/cu.usbmodem...
  target/xtensa-esp32s3-espidf/release/bruce-smart-home-capture`.
- For the Guition JC3248W535C, the same with `--features guition-jc3248w535c`.
- `python3 log.py [port]` shows a device's log without restarting it (needs pyserial). Not `espflash monitor`, which
  restarts it, or with `--no-reset` leaves it stopped in its bootloader. The CYD's port is `/dev/cu.usbserial-*`, its
  USB chip's, the Waveshare's `/dev/cu.usbmodem*`, the chip's own USB.

`./build-firmware.sh` builds every board's firmware, or `./build-firmware.sh <board>...` those named, and copies each,
as soon as it is built, as boards with the same chip build to the same path in `target/`, to the shared firmware
folder, as `bruce-smart-home/dev/bruce-smart-home-capture-<board>.elf`, see bruce-embedded-flash's
`bruce-embedded-firmware`, e.g. `~/Library/Application Support/bruce-embedded-std/firmware/` on macOS, where
the app's Devices page and bruce-embedded-flash's window find it. `./build-firmware.sh --path <board>` prints where a
board's goes.

In VS Code, Terminal > Run Task has **Device: Flash CYD**, **Device: Flash Waveshare**, **Device: Flash RLCD**,
**Device: Flash Guitron**, **Device: Log CYD** and **Device: Log Waveshare**, which find the port. Each flash task
builds its board's firmware with `build-firmware.sh`, then flashes its own file.

## Setting it up

The first time it starts, it makes its own WiFi network: join it and the config page opens. Enter your WiFi details,
then in the TeslaCapture section:

- **tesla_host**: the gateway's IP address.
- **tesla_email** and **tesla_password**: the gateway's customer login, as in the app. A password the gateway
  rejects is removed, so it is not tried again. Repeated failures lock out the customer login, for Home Assistant
  too.
- **tesla_schedule**: optional, when to poll, a cron spec with seconds first, `0 */5 * * * *` (every 5 minutes) if
  empty.
- **tesla_capacity**: optional, how many samples to keep in RAM, 48 (4 hours at 5 minutes) if empty. The SD card
  keeps the rest. The log shows the free heap after each poll.

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
