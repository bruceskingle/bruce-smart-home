# Capture Device Hardware

A capture device is a small ESP32 board with a screen, which fetches your data all the time, even while this computer
sleeps, keeps it on its SD card, and gives it to the app, see [Capture Devices](help/devices.md). These are the boards
there is capture firmware for.

## What a board needs

**An ESP32-S3 with PSRAM, and an SD card.** The PSRAM is the extra memory a device needs to keep its store, fetch from
the internet over HTTPS, and answer the app, all at once; boards without it run out. The SD card is where it keeps what
it fetched, so that nothing is lost when it restarts: without one it keeps only what fits in memory, until it restarts.

## The boards

| Board | Screen | Meets the minimum |
|---|---|---|
| Waveshare ESP32-S3-LCD-1.47B | 1.47" colour: the graph of the Powerwalls' gateway's power; tip the board, or press BOOT, for its status | Yes |
| Waveshare ESP32-S3-RLCD-4.2 | 4.2" black and white, readable in daylight: a clock with the latest numbers below; KEY for its status. It also logs the room's temperature and humidity, and has a speaker and a battery holder | Yes |
| Guition JC3248W535C | 3.5" colour touch screen: the graph; swipe or press BOOT for its status | Yes |
| Waveshare ESP32-C6-Touch-LCD-1.47 | 1.47" colour touch screen: the graph; swipe for its status | No: an ESP32-C6, without PSRAM |
| CYD (ESP32-2432S028R) | 2.8" colour: the graph | No: an ESP32 without PSRAM. It ran out of memory keeping its store, so it is no longer used as a capture device |

Each board's details, what it has and what is driven, are in the platform's
[Hardware](https://github.com/bruceskingle/bruce-esp-idf/blob/main/docs/Hardware.md) page.

## SD cards

Not every card works well: some, even of the same brand, do not come back after the device restarts without its power
being cut, e.g. after flashing, until it is unplugged. The cards which have been tried, and how they did, are listed on
the platform's [SD Cards](https://github.com/bruceskingle/bruce-esp-idf/blob/main/docs/Hardware.md#sd-cards) page.
Format a card as FAT32 (MS-DOS (FAT) in macOS's Disk Utility) before putting it in. If a device's screen says **SD card
missing: unplug, plug in**, unplug it and plug it in again.
