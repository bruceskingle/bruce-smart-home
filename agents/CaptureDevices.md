# Capture devices

Notes for coding agents working on the ESP32 capture devices, moved from AGENTS.md. Read them before
building, flashing or restarting a device, sending it a request, or reading its log.

- `capture/`: firmware for the ESP32 capture devices, built separately, see [capture/README.md](../capture/README.md).
  It logs with the `log` macros, never `println!`, which panics on a device with no computer reading its USB port,
  see the platform's CLAUDE.md in bruce-esp-idf.

- Read a device's serial log with `python3 capture/log.py <port>`, which leaves a board with a USB serial chip, e.g.
  the CYD, running, but restarts an ESP32-S3 on its own USB port, e.g. the RLCD and the Guitron (reset reason Usb). `espflash monitor` restarts
  it, and with `--non-interactive --no-reset` leaves it halted in its bootloader until `espflash reset`. The tasks
  "Device: Flash" and "Device: Log" are in `.vscode/tasks.json`.
- Before a request which changes a device, e.g. its settings, a restart, a release or a factory reset, check its state
  in the same command, e.g. `GET /api/info` says it is adopted, and stop if it is not as assumed: a "negative test" of a
  released device once wiped its Wi-Fi settings.
- Flash only with bruce-embedded-flash's `bruce-embedded-flash` (the cargo runner and the "Device: Flash" tasks use it),
  never espflash directly: it refuses firmware built for another board than the one on the port, from the record each
  image carries, and asks the owner which board it is if the firmware on it has no record. Never pass `--confirm-board`
  on the owner's behalf.
- Before flashing, check the board on the port is the one meant, by its MAC address, which is the device's ID in the
  app, e.g. `espflash board-info --port <port>`, which writes nothing but restarts it: the port names say only where a
  board is plugged in, and several boards may be.
- Flash or restart the devices one at a time, waiting until each polls again, so that one is always capturing, and
  away from the Octopus poll times, :29, :30, :59 and :00 past the hour.
- A device without an SD card keeps its records in memory, which a restart loses, so before flashing or restarting
  one, check that the app has read it lately, e.g. its last "GET /api/capture/..." in the profile's log, which it does
  only while a profile which is not offline is active.
