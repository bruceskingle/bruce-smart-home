# Plan: capture devices doing the work

Started 23 November 2026. Where the capture devices and the store are going, in stages, the known problems first, and
what is done. bruce-db's own design, which this follows, is its `docs/design.md`; how the app and the devices use the
store is [Store.md](Store.md).

**The aim:**
- The capture devices do all the polling, and the app polls a source only as a fallback, when no device has its data.
- The devices replicate the whole site's records between themselves, blobs aside, so that any one could serve the UI as
  web pages.
- Everything starts at once and catches up in the background: the app's first graph appears straight away and fills
  in as the data is read.

**Decided (Bruce, 23 November 2026):**
- **Peer access:** the app hands each device a site key when it adopts it, shared by the site's devices, and changes it
  when it releases one. Devices read each other's stores with it.
- **Discovery:** devices advertise themselves over mDNS as capture devices. The app finds them without an IP address
  being typed in, and they find each other the same way.
- **What a device keeps:** the whole site's records, without blobs, e.g. bill PDFs.
- **The minimum board** is an ESP32-S3 with PSRAM and an SD card, see [Capture Device Hardware](CaptureHardware.md).

## Stage 1: the known problems

All in bruce-db 0.3.0, bruce-esp-idf 0.3.0 and the capture firmware, linked while they are worked on, see the capture
firmware's git-ignored `.cargo/links.toml`.

| | Problem | Fix | State |
|---|---|---|---|
| 1.1 | A garbled line of the journal, e.g. bits flipped on the card, not UTF-8, stopped the whole store: no history, nothing stored (1.47B, 23 Nov) | Read lines as bytes, skip one which is not text or not JSON, with a warning | Done, tested, on the 1.47B |
| 1.2 | A flipped bit which still parses is stored as if right, e.g. a wrong digit | A CRC32 at the end of each line, checked when read; a line without one, written before, is accepted | Done, tested |
| 1.3 | Opening reads the whole journal, a day or more: 8.6 s on the 1.47B, 47 s and more on the Guitron, minutes after a missed compaction | The journal an hour a file, compacted every hour with `compact_through`, so that opening reads an hour or two; `format.json` 2 | Done, tested; on the 1.47B; a copy of the app's store opened in 0.1 ms after compacting |
| 1.4 | The change feed, for a peer behind what was compacted, reads every compacted file for every page: 4 to 7½ minutes a page on the Guitron and the RLCD, which blocks their plain HTTP the whole time | Keep compacted journal hours for three days, which a peer catches up from in sequence order; for one further behind, keep each compacted file's sequence numbers and offsets once read, so that only the first page scans | Done, tested: a page in 8 to 50 ms on a copy of the app's store, against 1.4 s scanning; on the Guitron, see below; "too far behind" waits for the full copy |
| 1.5 | The 1.47B's store was never compacted, so its journal held two days | It was not failing: a device compacted only the days before yesterday, and this store began on the 22nd. Compacts hourly now | Done |
| 1.6 | Some SD cards do not answer after a restart which does not cut the power | The firmware's own restarts unmount first; the screen and the app say to unplug | Done, on the 1.47B |
| 1.8 | Flashing a firmware file from the shared firmware folder with `--keep-as` emptied it, copying it onto itself | bruce-embedded-firmware's `keep_dev` leaves a file which is already the one kept | Done, flasher reinstalled |
| 1.9 | A device's plain HTTP is one task, so one slow page, e.g. the change feed's scan, blocks everything else on it, `/api/info` too, which the app then reports as unreachable, and new connections are refused (`httpd_accept_conn: error in accept (128)`) | With 1.4 pages are quick; still to decide: the change feed on a thread of its own, or a time limit on a page | Open |
| 1.10 | The Devices page's row for the Guitron stayed "Not reachable" after it recovered, until the page was opened again (23 Nov, after the app had napped) | Watch for it with the app in front; if it happens, the row's heartbeat stopped | Open |
| 1.7 | The app and the firmware on unreleased libraries | Release bruce-db, then bruce-embedded-std, then bruce-esp-idf, each 0.3.0, then the app and the firmware on them | Bruce, after review |

Devices: both 1.47s are flashed at will, the Guitron with care once there is a fix for 1.3 and 1.4, and the RLCD not
until the rest have run well, as the fallback.

## Stage 2: starting at once

- **On a device:** the store opens in a thread of its own; polls start at once, their records kept in memory until it
  is open, so that a start does not lose the first minute of samples, and the screen draws the latest at once.
- **In the app:** measure what holds up its first graph, then draw from what is already in its own store at once, and
  read the devices and sources in the background, the pages filling in as records arrive, through the signals they
  already read.
- **Measured, 23 Nov:** on 22 November Octopus's electricity took 68.7 s to load, Tesla's 0.27 s: Octopus's load
  fetches what is missing before it returns. **Started, 23 Nov (uncommitted):** `ElectricitySource::stored`, the same
  source given `CacheConfig::stored_only`, reads only the store; the Electricity page draws that at once, then the full
  load, with a line saying what is being fetched, and Cancel, in place of the overlay. Each range's last load is kept
  while the app runs, so going back to the page draws it at once. The meter readings, the Home Mini's and the bills
  pages do the same. Starting the modules was measured, and is quick.
- **The UI thread, sampled 23 Nov:** the spinning cursor was work on the window's thread: the agents guide decrypting
  secrets, so waiting for the Keychain prompt (6 s), myenergi syncing each record (6.5 s), the Home Mini's and the
  meter readings' store reads, the latest Tesla snapshot read over two days, the bitwise checksum, and the node file
  read for every put. All fixed; debug builds are signed with a stable identity, so the Keychain no longer asks for
  each build, see docs/developers.md. A start's own work on the window's thread is now about 1.5 s, the longest piece
  0.5 s. Next: EV charging, charging sessions and properties pages, and myenergi's save off the window's thread.

## Stage 3: discovery by mDNS

- **A device** advertises a service of its own, e.g. `_bruce-capture._tcp`, port 80, with TXT records of its ID (the
  Wi-Fi MAC), its board, whether it is adopted, and its site's ID once it is. The platform's mDNS responder already
  answers for its host name.
- **The app's Devices page** lists the capture devices found, adopted or not, and adopts one with a click. A device's
  address is no longer typed in, though it still can be, e.g. for a simulator on another computer.
- **Devices** find the site's other devices the same way, by the site in their TXT records.
- **Where:** the platform's responder, bruce-embedded-std's `mdns.rs`, answers only A queries for `<host>.local`, with
  `simple_dns`. A service needs PTR, SRV and TXT answers added there, with the TXT records set by the application, and
  finding peers needs queries sent and answers read on the same socket. The app needs an mDNS browser: a crate, e.g.
  `mdns-sd`, its licence and what it pulls in checked first, or the same `simple_dns` code.

## Stage 4: replication between devices

- **The site key:** given to a device by the app when it adopts it, with the site, kept in its settings, and changed
  on every device when one is released. A device's store is read, by the app or a peer, with a request signed with it.
- **Pulling:** each device pulls each peer's change feed, which it found by mDNS, and keeps its own cursor for each
  peer, as the app does; the peer keeps nothing, see bruce-db's design. Records are passed on, so all devices converge.
- **Blobs are not replicated** to devices, so that their cards do not fill.
- **A new device** gets the whole site with a full copy, bruce-db's step 7, then catches up.
- **The app's own records**, e.g. Octopus bills, go to the devices: open question, below.

## Stage 5: polling on the devices

- **Each internet source is polled by one device**, e.g. Octopus, myenergi, GloBird, chosen by the app, to spare their
  rate limits, see [agents/OctopusApi.md](../agents/OctopusApi.md); the others get its records by replication. A
  local source, e.g. the Tesla gateway, may be polled by several, so that any one keeps going alone.
- **The app polls a source only as a fallback**, when no device has a fresh record of it, by the source's own
  interval.
- Needs each source's polling code on the device, which the Octopus and myenergi captures already are, and the rest,
  e.g. bills, moved there.

## Stage 6: the UI from a device

Later: a device serves the app's pages, as plain HTML, CSS and JavaScript, from its own store.

## Open questions

- [ ] How the app's own records reach the devices: the devices pull the app's feed when it is running, found by mDNS
  too, or the app pushes them?
- [ ] How long compacted journal hours are kept: three days on a device, a week on a computer?
- [ ] Which device polls each internet source, and who changes it when that device is down?
- [ ] Whether the RLCD, with its black and white screen, stays a capture device once others have replaced it.
