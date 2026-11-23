# Design: capture devices replicating between themselves

Stage 4 of [the plan](CaptureDevicePlan.md), with the leader election stage 5 needs, written on 23 November 2026 for
review before any code. How the store replicates in general is bruce-db's `docs/design.md`, "Replication"; how the app
and the devices use the store is [Store.md](Store.md).

## What it is for

- **Every device holds the whole site's records,** blobs aside, so that any one can answer the app, or later serve
  the UI, and a device which was off catches up from the others.
- **The devices do the polling, not the app.** A source with history, e.g. Octopus, is polled by one device, its
  leader, and the rest get its records by replication; a source with no history, e.g. the Tesla gateway's local API,
  by every device. The app never gives the devices data, and polls only as a fallback.

## What there is already

- **Each device serves its change feed,** `GET /api/store/changes?since=<n>&limit=<k>`, on its server for slow reads,
  port 8080, signed with the app's key for that device, see bruce-esp-idf's `Adoption::is_signed`.
- **The app replicates from each device,** keeping a cursor and the device's store ID for each in
  `app.replication/<app node>/<device>`, so that a wiped card starts again from 0.
- **bruce-db replicates safely:** each line keeps its origin and the peer it came from; the newest version wins, so
  order and repeats do not matter; a record a node has already is not stored again, so nothing loops; and
  `changes(…, for_peer)` leaves out what came from or was made by the peer asking, so nothing goes back.
- **Catching up is cheap** within the three days of kept journal hours, and slow for only the first page further
  behind, see 1.4 of the plan.
- **Devices find each other** by mDNS, with their ID, firmware, site and bulk port in their TXT records.

## 1. The site key

Today each device has its own key, which only the app holds. For devices to read each other, they need a key they
share.

- **What:** 32 random bytes, made by the app for the site when it first adopts a device, kept in the profile's settings,
  encrypted, as the device keys are.
- **Given to each device** with its adoption, `POST /api/adopt`, and later with `POST /api/site-key`, signed with that
  device's own key, so only the app can change it. The device keeps it in NVS, in `device`, beside its own key.
- **Used for reading only:** a device accepts a request for `/api/store/changes` and `/api/store/info` signed with
  either its own key, the app's, or the site key, a peer's; everything else, e.g. its settings, still needs its own.
- **Changed when a device is released** from the site, so that it can no longer read the others: the app makes a new
  key and sends it to each device it can reach. One it cannot reach, e.g. one which is off, gets it when the app next
  reaches it; until then it cannot read its peers, nor they it, which is safe, and the app sees it on the Devices page.
- **Every capture device is given it,** as every one is an ESP32-S3 with PSRAM, see "Small boards" below.

*Alternative considered:* a key for each pair of devices, so that releasing one changes nothing for the rest. With n
devices that is n(n−1)/2 keys, each given to two devices, which the app must keep in step; for a handful of devices in
one house the shared key is simpler, and as safe, as a released device is one the user chose to remove.

## 2. Finding peers

- **Each device browses for `_bruce-device._tcp`** every 5 minutes, and at start, keeping those with its site in their
  TXT records, their IDs, addresses and bulk ports. bruce-embedded-std's responder learns to ask as well as answer: a
  query from its own socket, and the replies read as the app's `discovery.rs` reads them.
- **A peer's address changes** are followed by the next browse, as the app follows them.
- **A device with no peers** does nothing more, so one device alone is as today.

## 3. Pulling

A feature of the capture firmware, **Replication**, which, every minute, for each peer:

1. **Asks its change feed:** `GET http://<peer>:<bulk port>/api/store/changes?since=<cursor>&limit=200&for=<own ID>`,
   signed with the site key. `for` is new: the device passes it on as `changes(…, for_peer)`, so that the peer leaves
   out what came from or was made by the device asking.
2. **Puts each change** with `put_from(item, peer)`, so its line says where it came from, which passes it on to other
   peers but not back.
3. **Keeps its cursor** for the peer, with the peer's store ID, in a file of its own on the SD card,
   `/sd/replication.json`, not in the store, as a cursor is the device's own and must not replicate; a peer whose store
   ID has changed is read again from 0.
4. **Pages until it is caught up,** then waits for the next minute.

- **Blobs are not pulled:** a record which refers to a blob, e.g. a bill's PDF, replicates; the blob does not, so
  that the cards do not fill. A device which is asked for a blob it does not have says so.
- **One peer at a time,** on the feature's own thread, so that the polls and the screen are not held up; a slow peer is
  given up on after a minute and tried again next time.
- **Load:** with 4 devices, each asks 3 peers a minute, 12 small requests a minute across the site; the bulk server
  takes 3 at once, so peers and the app fit.

## 4. A device further behind: the full copy

A peer's cursor older than the kept journal, e.g. a new device, or one off for more than three days, today reads the
DataSets' files, which is slow for the first page. That is acceptable at first. Later, bruce-db's step 7, the full copy:

- **The feed answers "too far behind"** when `since` is older than the journal kept.
- **The peer then copies** the DataSets' files in their own order, a page at a time, with the node's latest sequence
  number as a watermark, then catches up from it, see bruce-db's design, "The journal by the hour".

## 5. The app

- **Reads one device, not all:** as every device has every record, the app reads the device most up to date, by its
  latest sequence number in `/api/store/info`, and the others only if it cannot reach that one. Until replication is
  on every device, it reads all, as now.
- **Gives the site key** at adoption, and changes it at release, see 1.
- **Shows replication** on the Devices page: each device's peers, and how far behind each is.

## 6. Leader election, for stage 5

Who polls each source with history, without a server, in the store itself, which replicates:

- **A lease record** for each source, `site.lease/<source>`, e.g. `site.lease/octopus`, with the device holding it and
  when the lease ends, 15 minutes after it was renewed.
- **The holder renews it** every 5 minutes, while it is polling the source successfully.
- **Another device takes it** when the lease has ended, after a wait of its own, from its ID, so that two do not try at
  once, by putting a lease with itself as holder.
- **Two which do put leases at once** both replicate, and bruce-db's rule, newest version wins, and of equal times the
  lowest origin, picks one; the other sees it within a minute and stops. A poll or two made twice meanwhile is harmless,
  as records are keyed by the time they are of.
- **The clock:** devices set it by SNTP; a lease of 15 minutes is far longer than any difference between their clocks.
- **A source with no history,** e.g. the Tesla gateway, has no lease: every device polls it.
- **The app polls a source itself** only when no record of it is newer than a few of its intervals, e.g. no device is
  up, and never holds a lease.

## Small boards

A board without PSRAM, e.g. the Waveshare ESP32-C6-Touch-LCD-1.47 or the CYD, has no room for HTTPS or a replica, so
it is not a capture device: the capture firmware is built only for ESP32-S3 boards with PSRAM and an SD card, see
[Capture Device Hardware](CaptureHardware.md), and every capture device is a full member of the site. A small board
may later be a display-only device, which reads a capture device's latest records as the app does, keeps no store,
and is given no site key, but its own key, as now.

## Order of work

1. **The site key:** made and given at adoption, changed at release, accepted for reading by the devices.
2. **`for` on the change feed,** on the devices.
3. **Browsing on the devices,** in bruce-embedded-std's responder.
4. **The Replication feature,** tried first with simulated devices, several of which can run on one computer, each
   with its own store, then on the 1.47B and the Guitron, then the RLCD.
5. **The app reading one device,** and the Devices page showing replication.
6. **Leader election,** then stage 5, the sources moved onto the devices one at a time, Octopus first.
7. **The full copy,** in bruce-db, when a device first needs it.

## For you to decide

1. **One site key, changed at release** (recommended), or a key for each pair of devices?
2. **Pulling every minute** (recommended), or less often, e.g. every 5 minutes, which is less traffic but leaves a
   device further behind when one goes off?
3. **The app reading one device** once all replicate (recommended), or all, as now, which costs more but needs no
   choosing?
4. **Leases of 15 minutes, renewed every 5** (recommended), or shorter, so that a source is taken over sooner when its
   leader dies, at more traffic?
