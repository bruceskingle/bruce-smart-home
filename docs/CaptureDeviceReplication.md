# Design: capture devices replicating between themselves

Stage 4 of [the plan](CaptureDevicePlan.md), with the leader election stage 5 needs, written on 23 November 2026, and
agreed the same day, see "Decided". How the store replicates in general is bruce-db's `docs/design.md`, "Replication";
how the app and the devices use the store is [Store.md](Store.md).

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

## 1. Who may do what: the domain's Security Contexts

Written on 24 November 2026, as the first use of bruce-db's Security Contexts, see its design, "Security Contexts and
Principals", which is the target for who may do what in any store; this is the part built now. It replaces a shared
secret domain key, coded the same day and never flashed. Until now each device has had a key of its own, which only the
app which adopted it holds, so one app manages it, and nothing else may read it. What is wanted, from Bruce:

- **Co-owners,** one or two people, e.g. Bruce and his wife, genuine equals, each with full control, whichever app
  they use.
- **Admins,** in a larger organisation many, who manage the devices but not who else may, added and removed by the
  co-owners, one at a time, whenever need be.
- **Readers,** who may only read, e.g. Jeff reading Bruce's devices, and Bruce reading Jeff's, each a co-owner of his
  own domain.
- **The devices,** which read each other's stores, see 3, and nothing more.

The domain's records need no encryption: they are the household's own, on its own devices. The SCs' keys are made all
the same, so that it could be turned on, as bruce-db allows for each SC.

### The domain's principals and SCs

- **Each person is a user principal of each domain they belong to,** e.g. Bruce of his own, a co-owner, and of Jeff's,
  a reader, two principals, made for each, which nothing links, as each domain is a domain of the app, shown one at a
  time. Their private keys are in a **key file**, protected by a password, which they may keep on a USB stick, and
  which the app asks for when the user's key is needed, e.g. to add a member, unless they chose to have this computer
  remember it, with a key in the system's credential store; one file may hold a user's principals in several domains,
  each with keys of its own.
- **Each installation of the app is a system principal of each domain,** called e.g. "Bruce's Smart Home on Bruce's
  MacBook", made for the domain, its private keys in the domain, encrypted, as its other secrets are. It has no role of
  its own: it acts for a user, as their **actor**, see bruce-db's "Subjects and actors", signing alongside them while
  they are present, and, while they are away, e.g. replicating every few minutes, alone, by a **delegation** the user
  signed, up to a role, so that the authority is always the user's. **So is each device,** its private keys in its NVS,
  never sent, its public keys in `/api/info`, acting for itself, by memberships of its own.
- **A domain, bruce-db's, is founded only when a user asks,** "Found This Domain" on the Devices page,
  which a new domain asks for at once, never by the app on its own: the app asks for their key file, or makes one,
  makes the user's keys, the domain's ID from them, then the user's principal in it, its founder, the app's, and the
  SCs, signed by the user, and delegates to itself for them. As a domain's ID is its namespace, a new domain has no
  store until its domain is made, `awaitingFounding`, and a domain made before domains, with a store already, is moved
  to it, after the app has read everything its devices have, and its devices' stores are moved too, then the app is
  restarted, as parts of it hold the old store; the store of the domain before is left as it was. Until a domain has a
  domain, or has joined one, the app goes on using each device's key, as before.
- **The domain's SCs:**
  - **the domain itself,** bruce-db's root SC, whose Admins are the co-owners, as equals, each of whom can add or
    remove the other, and later its backup Admin, whose key file is kept offline, to give a co-owner new keys if
    theirs are taken, see bruce-db's "Domains" and "Replacing a key";
  - **`bruce-smart-home`,** the app's SC, of which the domain is an Admin, see bruce-db's "The SC graph";
  - **`control`,** under it, whose Write members are the admins;
  - **`data`,** under it too, which every record is in, the domain's default SC, whose Write members are the devices
    and whose Read members are the readers.

  A co-owner is thereby an Admin of all four; an admin may change the devices but add no one; a device or a reader
  may read the store and nothing more.
- **On a device, a signed request** may do what its subject's role allows, or, if delegated, the lower of that and the
  delegation's:

  | | Read its store | Read its info, config and log | Change its settings, restart, release | Add or remove members |
  |---|---|---|---|---|
  | Co-owner, Admin of the domain | Yes | Yes | Yes | Yes, as records, see below |
  | Admin, Write in `control` | Yes | Yes | Yes | No |
  | Reader, Read in `data` | Yes | No | No | No |
  | Device, Write in `data` | Yes | No | No | No |

  A user's app, delegated, e.g. Write in `control` and `data` for a co-owner, so that it can manage and read the
  devices while the user is away, but never add a member, which needs the user's own signature.
- **Changes go over HTTPS,** as now, signed, in place of the `Bearer` key; reads over plain HTTP, also signed, as each
  costs a device far less RAM. A device checks the signatures, a few milliseconds each on an S3, against the
  principals' records, and the roles, from the membership and delegation records, which it keeps in memory, and reads
  again when such records arrive.
- **Adding or removing a member** is a co-owner putting a membership record, signed by them, their key file unlocked,
  with their app as actor. It reaches every device by replication, within minutes, including one the app cannot
  reach; the app also sends it straight to each device it can, `POST /api/store/security`, so as not to wait. Nobody
  else's key changes. A removal cannot be undone by a stolen device putting the older record again, as a security
  record's version is when it was signed, and each change to a membership is a record of its own, judged in the order
  they were signed, see bruce-db's "Versions of signed records" and Membership.
- **A reader is never given the domain's security records:** a device's change feed and full copy leave them out for
  any request but one with the adopting app's key, or one signed for an Admin of the domain, see bruce-db's "Servers
  and clients".
- **Principals belong to the domain,** a domain in bruce-db's words, whose ID is in theirs: each, with its keys and what
  is said of it, e.g. its `displayName`, is a DataSet of the domain, its record signed into it by a co-owner, who also
  gives a device its `displayName`. Sharing records between domains, which bruce-db's design has for later, is not
  needed here: a person who reads another's domain joins it.
- **Later, every record** is signed by its subject, or its actor, and notarised by the device it is written to, its
  edge server, which checks the signer's time against its own clock; every device it is replicated to checks it all
  again, see bruce-db's "Signed records". Until then, a device trusts the records of the member devices it replicates
  from.

### Adoption, and the devices adopted before

- **Adopting** is by the code on the device's screen, as now, giving the device a key, then its domain's ID, as a
  setting, the domain's, once the domain has one. The device makes its principal in that domain when its store first
  has it, keeping its private keys in NVS, and gives its record and first key, signed by itself, in `/api/info`, as
  `security`. A co-owner then admits it to the domain, "Add Devices to the Domain", signing its record into the domain,
  naming it, and making it a Write member of `data`.
- **Moving it over:** in a request of its own, made with its key, so that a device adopted before is moved over the
  same way, the app gives the device the domain's security records, as its store has none yet, in batches small enough
  for it to take, `POST /api/store/security {"items": [{"hk", "sk", "t", "o", "a"}]}`, each as the app's store keeps
  it, which the device puts in its store, if it is one of its domain's security DataSets, and counts only if it is
  signed as it should be.
- **A device adopted before** keeps its own key, which the app holds, until the app has moved it over. The device then
  takes signed requests, which the app makes once the device's `/api/info` says it has the domain's records,
  `security.ready`. The device goes on taking the key until it is released, as a request in any other scheme than
  `Bruce-Signed` is checked by the key, see bruce-esp-idf's `access.rs`.
- **Releasing a device** should remove its membership, so that the others no longer read it, nor it them; not yet
  built. Its principal in the domain is kept, unused, until its store is given another domain.

### Joining a domain

- **Someone joins the domain,** e.g. Bruce's wife, as a co-owner, an admin, or Jeff, as a reader, by becoming a
  principal of it, as bruce-db's design says:
  1. A co-owner saves an invitation, "Invite Someone…", with the domain's ID and name, none of it secret.
  2. "Ask to Join a Domain…" opens it: their app makes their principal for the domain, with the domain in its ID, in
     their key file, and its own, kept until the reply comes, and saves a request: both principals' records and first
     keys, their delegation to the app, and their `displayName`, none of it secret either.
  3. "Let Someone Join…": a co-owner, their key file unlocked, signs both principals into the domain, puts the records
     and the membership, Admin of the domain, Write in `control`, or Read in `data`, sends them straight to the devices
     it can reach, and saves a reply: the domain's ID, and the devices' addresses and certificates' fingerprints.
  4. "Join a Domain…": the joining app makes a domain for the domain, and reads the devices, signing as the user's
     actor.

  The trial of 25 November 2026 did this, see the order of work.
- **The joining app holds no device keys:** a device in its domain has none, and every request to it is signed.
- **The same user on another app** needs no co-owner: they sign that app a delegation, as the authority is theirs.
- **Saving and opening these as files by hand** comes first; sending them by email is on the backlog.
- **Reaching another domain's devices,** e.g. Jeff's from Bruce's home, needs a network path, e.g. a VPN, see the
  Powerwall 3 plan; the memberships only say who may.

### Built now, and later

- **Built on 24 November 2026:** in bruce-db's `security` module, user and system principals, a key's `until`, SCs,
  signed memberships, delegations, and requests signed by their subject and actor, checked by role; in bruce-esp-idf,
  the `Authoriser`; in the capture firmware, the device's principal, its checks, and `POST /api/store/security`; in the
  app, the user's key file, adding devices, moving them over, signing as the installation for its user, and joining by
  files, on the Devices page. A delegation is for the whole domain, up to Write, not for one SC.
- **Built on 25 November 2026,** bruce-db's step 12: the domain as a domain, its ID from its founder's first key, the
  domain's store moved to it; principals as DataSets of the domain, admitted by a co-owner, with keys committing to
  their next and names as records of their own; the SC graph, with the app's SC; memberships kept as a history and
  judged in the order they were signed, in place of each device's file of them; a key file holding a principal in
  each domain; joining by an invitation first; and the security records left out of a reader's feed.
- **Later,** with bruce-db: records signed by their subjects, notarised and checked when replicated; fetching a
  principal's record from its source; authentication; wrapped keys, encryption and rolling keys, if ever wanted here.
- **What it does not cover:** a co-owner removed by another may have kept the domain's records, as they are not
  encrypted.

*Alternatives considered,* on 24 November 2026:

- **A shared secret domain key,** which the devices read each other with: for one app only, as two would give the
  devices each other's old keys.
- **Secrets worked out from a co-owners' secret,** each device's key and a read key by generation: a device must hold
  any key it checks a request with, so a stolen one could act as an admin to the others, and removing one admin means
  a new key for every admin.
- **A signed member list with roles, but no SCs:** close to this, but a model of its own, which bruce-db's Security
  Contexts would later have to replace.
- **The app as the co-owner,** a principal for each domain, which made the domain by itself: the authority must be the
  user's, whichever app they use, and a domain is made only when they ask.
- **A person as an SC,** whose Admins are their apps: a person is a principal, as every actor is, with keys of their
  own; their apps act for them by delegation.

## 2. Finding peers

- **Each device browses for `_bruce-device._tcp`** every 5 minutes, and at start, keeping those with its site in their
  TXT records, their IDs, addresses and bulk ports. bruce-embedded-std's responder learns to ask as well as answer: a
  query from its own socket, and the replies read as the app's `discovery.rs` reads them.
- **A peer's address changes** are followed by the next browse, as the app follows them.
- **A device with no peers** does nothing more, so one device alone is as today.

## 3. Pulling

A feature of the capture firmware, **Replication**, which, every minute, for each peer:

1. **Asks its change feed:** `GET http://<peer>:<bulk port>/api/store/changes?since=<cursor>&limit=200&for=<own ID>`,
   signed by the device, as its principal, see 1. `for` is new: the device passes it on as `changes(…, for_peer)`, so
   that the peer leaves out what came from or was made by the device asking.
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

A peer's cursor older than the kept journal, e.g. a new device, or one off for more than three days, read the
DataSets' files in 0.3.0, which was slow for the first page. As every device's first pull from a peer is that far
behind, bruce-db's step 7, the full copy, comes first, and is built in bruce-db 0.4.0:

- **The feed answers "too far behind"** when `since` is older than the journal kept, or the journal kept has a gap
  after it, e.g. an hour lost from the card.
- **The peer then copies** the DataSets' files in their own order, a page at a time, with the newest sequence number
  compacted as a watermark, then catches up from it, see bruce-db's design, "The journal by the hour". A copy which
  outlasts the journal kept expires, and is started again.
- **On the devices:** `/api/store/changes` says when the peer is too far behind, and `GET
  /api/store/copy?token=<token>&limit=<k>&for=<own ID>`, on the bulk port, signed as the feed is, gives a page of the
  copy and the token for the next, or the cursor to catch up from.

## 4b. Repair

A line damaged on a device's card, e.g. bits flipped, fails its checksum, and the record is missing there, while its
peers have it. The feed will not send it again, as the device's cursors are past it, so the device asks for it:

- **The store notes where** it found each damaged line, a DataSet's file or a journal hour, in a file of its own,
  which does not replicate, `Store::damaged`, see bruce-db's design, "Repair".
- **The Replication feature asks a peer for that place:**
  - for a DataSet's file, a copy of that file alone, `/api/store/copy?file=<file>`;
  - for a journal hour, the peer's changes again from the cursor the device had for that peer when the hour began,
    for which it keeps each peer's cursor at the start of each hour, for the three days the journal is kept.
  
  Neither passes `for`, as what the device made itself, and the peer leaves out for it, is what it may have lost.
- **What comes back is the same version** as the damaged line, which bruce-db 0.4.0 stores in its place, so the next
  compaction writes the file whole. The place is marked repaired once a peer has answered, and the Devices page shows
  a device's repairs, as damage is the first sign of a failing card.
- **A device alone** has no peer to ask; its damaged records stay missing, as today, and the app, which also has them,
  is the next to try, later.
- **Damage which is never read,** e.g. in a file no query reaches, is not found this way. A later check could compare
  each file's keys and versions with a peer's, now and then; not until repair has been used.

## 5. The app

- **Reads one device, not all:** as every device has every record, the app reads the device most up to date, by its
  latest sequence number in `/api/store/info`, and the others only if it cannot reach that one. Until replication is
  on every device, it reads all, as now.
- **Manages the members,** see 1: adopting and releasing devices, inviting co-owners, admins and readers, and
  removing them.
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
and is not a member of the site, but read with its own key, as now.

## Order of work

1. Done, 24 November 2026: **The full copy,** in bruce-db, 0.4.0, first, as a device's first pull from a peer starts at
   0, which is behind the kept journal of any peer older than three days, so every new pairing needs it, see 4.
2. **The site's Security Contexts,** see 1, replacing the shared secret site key coded on 24 November 2026, taken out
   again the same day before it was committed:
   - done, 24 November 2026: bruce-db's `security` module, its first part;
   - done, 24 November 2026, built but not yet tried on a device: the devices' side, bruce-esp-idf's `Authoriser`,
     `access.rs`, which the capture firmware's `site.rs` is, and `POST /api/store/security`;
   - done, 24 November 2026, tested only by a unit test so far: the app's side, `src/site_security.rs`, a device
     moved over once a run, from replicate, with its key, and its requests signed from then on; but the app made the
     site and was its co-owner by itself, which 1 now replaces;
   - done, 24 November 2026: 1 as now written, in bruce-db (a key's `until`, user and system principals, subjects and
     actors, delegations), the firmware (checks by subject and delegation, delegation records, 403 for a signer whose
     role does not allow a request, 401 for one not known) and the app (`src/user_key.rs`, the key file;
     `src/site_security.rs`, the installation's principal, "Make a site", adding devices, joining by files; the Site
     section of the Devices page, `src/views/site_access.rs`);
   - done, 24 November 2026: the trial with two virtual Guitrons, `src/site_trial.rs`, run by hand, three steps each
     in a process with an app's own data home: Jeff asks to join; Bruce's app adopts the devices, reads them with
     their keys, makes the site, adds and moves them over, reads them signed and manages them, a stranger is refused,
     and Jeff is admitted as a reader; Jeff's app joins, reads both devices signed for him with no keys, and is
     refused their settings, 403. It found that a reader refused the settings was answered 401, which made the app
     drop its signature for the key, now 403;
   - done, 25 November 2026: bruce-db's step 12, see "Built now, and later", and the trial again, four steps: Bruce's
     app adopts two virtual Guitrons, reads them with their keys, founds the domain, which moves its store and
     theirs to the domain's ID, its DataSets with it, admits them and moves them over, reads them signed and manages
     them, a stranger is refused, and it saves an invitation; Jeff's app asks to join from it; Bruce admits him as a
     reader; Jeff's app joins, reads both devices signed for him, is given none of their security records, and is
     refused their settings, 403. It found that the records had grown past what a device takes in one request, 8 KB,
     so they are sent in batches;
   - not built yet: "remember on this computer" for the key file, and removing a released device's membership;
   - then the Guitron, with the app, flashed on 24 November 2026 with the firmware before these, so to be flashed
     again.
3. Done, 24 November 2026: **`for` on the change feed,** and the full copy's endpoint, on the devices.
4. **Browsing on the devices,** in bruce-embedded-std's responder.
5. **The Replication feature,** with repair, tried first with simulated devices, several of which can run on one
   computer, each with its own store, then on the 1.47B and the Guitron, then the RLCD.
6. **The app reading one device,** and the Devices page showing replication.
7. **Leader election,** then stage 5, the sources moved onto the devices one at a time, Octopus first.

## Decided

Bruce's choices, 23 November 2026, each as recommended:

1. **One site key,** changed when a device is released, not a key for each pair of devices; superseded on 24 November
   2026 by the site's Security Contexts, see 1, as Bruce wants co-owners, admins and readers.
2. **Pulling every minute.**
3. **The app reading one device** once all replicate.
4. **Leases of 15 minutes, renewed every 5.**

And on 24 November 2026, for 1:

5. **Every actor is a principal,** user or system, behaving the same, with a signing and a key exchange pair; a key
   ends with one date, `until`, with no certificates, expiry or revocation lists.
6. **A user's keys are in a key file,** or on a secure USB key device, behind a password, for now.
7. **The authority is the user's:** requests and records have a subject and an actor, both signing, the terms of
   RFC 8693; an app acting while its user is away does so by a delegation the user signed.
8. **An app installation has a principal of its own,** as it acts autonomously.
9. **A site is made only when the user asks.**
10. **Each site stays a domain,** a user's principal being read-only in another's site.
11. **Later, every record is signed by its subject and notarised by its edge server,** whose time, within a margin of
    the signer's, is the record's for every check, and checked again by every node it is replicated to.

Boards below the minimum are not capture devices at all, see "Small boards".
