# Your Data
Bruce's Smart Home keeps what it has in two directories in your home directory, or in the data directory, see
[Profiles](profiles.md#moving-the-data-directory), kept apart so that your settings and secrets are never mixed with the
data, which may be large, copied or deleted:

- ```.bruce-smart-home```, your settings and secrets:
  - ```profiles.json```, your profiles, their settings, and the credentials of the data sources, see
    [Profiles](profiles.md).
  - ```tokens```, each profile's modules' sessions and refresh tokens, a file ```profile-module.json``` each. Reusing
    them avoids logging in each time, which data sources limit, so that you do not see "Too Many Requests" errors.
    They are sensitive: do not share them with anyone.

  The secrets in these, the API keys, passwords, login emails, capture devices' keys and tokens, are encrypted, with a
  key kept in your computer's own store of secrets: the Keychain on macOS, the Credential Manager on Windows, and the
  Secret Service (GNOME Keyring or KWallet) on Linux, under "Bruce's Smart Home". So a copy of the files, for example
  in a backup, does not give them away, and they can only be read on the computer which has the key. On macOS the
  Keychain may ask whether Bruce's Smart Home may use the key, the first time, and after it is updated.
  - ```node```, this installation's identity in the store, and ```window-state.json```, where the window was.
- ```.bruce-smart-home-data```, the data, which holds no secrets:
  - ```store```, each profile's data, a directory for each, named by its site's ID: everything fetched from the data
    sources, as they sent it, and what the capture devices took. Much of it, e.g. a capture device's snapshots or a
    source which only keeps a few weeks, can not be fetched again, so it is not a cache to delete lightly.
  - ```logs```, each profile's log, ```profile.log```.

Keeping a copy of what has been fetched makes the app faster, and reduces the load on the services it reads, most of
which limit how many requests can be made in a time.

## The store
Each profile's data is kept in a store, [bruce-db](https://github.com/bruceskingle/bruce-db), which is designed to be
copied between the app and the capture devices, and later to a cloud database such as Amazon DynamoDB. Every record has
a ```Hash Key```, which names the set of data it is in, e.g. a meter's readings, and a ```Sort Key```, which orders it in
the set, e.g. a reading's time, and a version, the newest of which is the record. A profile's directory holds a
```journal``` of every record as it arrives, and, folded into them a day or so later, a directory for each set of data,
of files of its records in order of their sort keys, a file for each day of a series. Account and meter numbers are
replaced by hashes in the keys and the names of the files.

## Before version 0.5.0
Marco Sparko kept the profiles in ```.marco-sparko```, and everything else in a directory ```.marco-sparko-cache```.
The first time version 0.5.0 runs, the profiles, tokens and logs are copied into the new directories, and each profile
is offered an upgrade the first time it is opened, which copies its data from ```.marco-sparko-cache``` into its store.
The old files are left as they were.
