# bruce-smart-home
# Release Process

Each release is developed on a branch called `vX.Y.Z`, which is squash merged to `main` and tagged `releaseX.Y.Z`.
The installers are built on GitHub and attached to a draft release, which is published once they have been tried.
Then the branch for the next version is started.

The steps are run with [bruce-method-release](https://github.com/bruceskingle/bruce-method-release), which releases
the libraries too, the same way; its README describes the method. Install it with `cargo install bruce-method-release`,
or, for a version not yet published, from its checkout with `cargo install --path ~/git/bruce-method-release`. It needs
the GitHub CLI, `gh`, signed in with `gh auth login`. The app's settings for it, its workflow and the installers it
builds, are in [Cargo.toml](../Cargo.toml), under `[package.metadata.bruce-method]`.

The version comes from Cargo.toml, which `start` sets at the beginning of each release, so it never needs to be typed.
Each command checks that the repo is in the state it expects, says what it will do and asks once before changing
anything, and stops with an explanation at the first problem. Run them in the checkout, e.g. in
`~/git/bruce-smart-home`.

```
bruce-method-release status
```

shows where things are: the branch, version and uncommitted changes, the commits last tried and tagged, the GitHub
release and the latest workflow runs.

The [Release workflow](../.github/workflows/release.yml) is made by bruce-method-release's `setup`, from its template
for Dioxus desktop apps, so change the template, not the workflow; `check` fails while they differ. The app's own step,
building the virtual capture device before the installers, is its hook,
[.github/actions/before-bundle/action.yml](../.github/actions/before-bundle/action.yml), which is the app's own file.
After updating Dioxus, run `bruce-method-release setup` to build with the new version of its CLI.

## 0. Release the Libraries

The app's libraries, e.g. bruce-graphql, bruce-db and bruce-embedded-flash, are separate repositories, published to
crates.io, and the app is built with their published versions, as the workflow checks out only this repo. While one is
being changed with the app, the app takes it from its checkout next to this one, by path or a link, so release those
first:

```
bruce-method-release dependencies
```

lists them in the order to release them, a tick for each one which is ready and a cross and what to do for each which
is not. Releasing a library makes the app take it from crates.io. `release` stops while the app still takes a library
by path or links it. `try` builds with the libraries the app links, each checked out from its branch beside the app,
so commit and push them first; it stops while the app takes one by path.

The capture firmware, in `capture/`, is a project of its own, which bruce-method-release's `dependencies` does not
see, though `link` and releasing a library update it: it takes bruce-embedded-std, bruce-embedded-sim and bruce-esp-idf
from crates.io. While one of them is changed with the firmware, it takes it by path, and the workflow must check its
branch out next to this repo, so that `try` can build the virtual capture device, see `capture/build-sim.sh`: release
the library, and take it from crates.io again, before the app's release.

The manual commands for some steps are in the collapsed sections, in case the tool cannot be used. They need the
version in `RELEASE`, e.g. `export RELEASE=0.6.0`.

## 1. Try the Installers

```
bruce-method-release try
```

Builds the installers from the release branch's last commit before anything is merged, to try them on other machines.
It tags the commit `testX.Y.Z` (moving the tag if it exists), runs the workflow, watches it (about 30 to 40 minutes,
the Intel Mac is the slowest) and downloads the installers, each in a folder named after its build:

```
~/Downloads/bruce-smart-home-X.Y.Z-test/macos-aarch64/BruceSmartHome_X.Y.Z_aarch64.dmg      Apple silicon Mac
~/Downloads/bruce-smart-home-X.Y.Z-test/macos-x64/BruceSmartHome_X.Y.Z_x86_64.dmg           Intel Mac
~/Downloads/bruce-smart-home-X.Y.Z-test/windows-arm64/BruceSmartHome_X.Y.Z_arm64-setup.exe  ARM PC
~/Downloads/bruce-smart-home-X.Y.Z-test/windows-x64/BruceSmartHome_X.Y.Z_x64-setup.exe      Intel or AMD PC
```

Each run of `try` replaces them. Run it again after fixing anything. A successful try of the branch's last commit is
also its CI run, which `release` checks for, so the last thing done before releasing is always a try. The test tag is
deleted by `release`.

If a build fails, it shows the commands to see why and to re-run the failed jobs.

Ctrl+C stops watching but not the run. To pick it up again, e.g. after a re-run of failed jobs, and download the
installers when it finishes:

```
bruce-method-release watch
```

To try the installer on another machine, see
[Trying the Installer on Another Machine](#trying-the-installer-on-another-machine).

<details>
<summary>Manual commands</summary>

```
cd ~/git/bruce-smart-home
git tag -f test$RELEASE
git push -f origin test$RELEASE
gh workflow run release.yml --ref v$RELEASE -f tag=test$RELEASE
```

`--ref` runs the version of the workflow on the release branch, so changes to it can be tried before they are merged.
Wait a few seconds for the run to start, then watch it and download the installers:

```
RUN=$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch $RUN
gh run download $RUN --dir ~/Downloads/bruce-smart-home-$RELEASE-test
```

Each build's installers are kept as an artifact named `macos-aarch64`, `macos-x64`, `windows-x64` or `windows-arm64`
for 90 days. Particular builds can be downloaded with `-n`, e.g. while the slower builds are still running (if `gh`
refuses because the run is in progress, wait for it to finish):

```
gh run download $RUN -n windows-arm64 -n macos-aarch64 --dir ~/Downloads/bruce-smart-home-$RELEASE-test
gh api repos/bruceskingle/bruce-smart-home/actions/runs/$RUN/artifacts --jq '.artifacts[] | "\(.name) \(.size_in_bytes)"'
```

On a Windows VM without bruce-method-release, in PowerShell:

1. Install `gh` with `winget install --id GitHub.cli`, or from https://github.com/cli/cli/releases (the "windows
   arm64 installer" or "windows amd64 installer", which is the `.msi`). Then open a new PowerShell window and sign in
   with `gh auth login`.

2. Find the run ID, the number which identifies the run of the workflow. It is shown in any of these places:
   - On the Mac, in the output of `try`, `release` or `watch`: `bruce-smart-home: watching run 40215599738, ...`
   - In the first line which `gh run watch` shows, after "Release ·": `* v0.4.4 Release · 40215599738`. The other
     numbers it shows, `(ID 121252356794)` after each job, are job IDs, which are not what is needed here (they are
     only used to get one job's log, see below).
   - In the address of the run's page on the Actions tab: `.../bruce-smart-home/actions/runs/40215599738`
   - Listed with the recent runs, on either machine, with
     `gh run list -R bruceskingle/bruce-smart-home --workflow release.yml --limit 5`
     (the ID is the long number in the ID column, the newest run is first).

3. Set `$RUN` to it. The variables set on the Mac are not set in PowerShell, so this is always needed there. With the
   number from step 2 in place of 40215599738:

   ```
   $RUN = "40215599738"
   ```

   or, to use the latest run without looking up its number:

   ```
   $RUN = gh run list -R bruceskingle/bruce-smart-home --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId'
   ```

   `echo $RUN` shows which run it is set to.

4. Download the installer, `windows-arm64` on an ARM PC or `windows-x64` on an Intel or AMD PC:

   ```
   gh run download $RUN -R bruceskingle/bruce-smart-home -n windows-arm64 --dir $HOME\Downloads\bruce-smart-home-test
   ```

   `-R` means these commands work in any directory. The setup.exe is in `Downloads\bruce-smart-home-test`.

The artifacts can also be downloaded from the run's page on the Actions tab of the bruce-smart-home repo, under
**Artifacts** at the bottom of the Summary (only the name is a link).

If a build fails, see why with the first command, or the second if that prints nothing (the job ID is shown next to
the job by `gh run watch`), and re-run the failed jobs with the third:

```
gh run view $RUN --log-failed
gh api repos/bruceskingle/bruce-smart-home/actions/jobs/<job id>/logs | grep -B2 -A10 -i "error\|panicked"
gh run rerun $RUN --failed
```

</details>

## 2. Release, up to the Draft

Write the release's section of the [ChangeLog](ChangeLog.md) first, see [Release Notes](#release-notes). Then:

```
bruce-method-release release
```

makes the release's checks, and stops if any fails: the branch, version, nothing uncommitted or unpushed, no library
linked or taken by path, the ChangeLog's section, `cargo fmt`, `cargo clippy` with no warnings, the tests, and a
successful try of the last commit. Then it lists what it will do, and asks once:

1. Creates a pull request from the release branch and squash merges it to `main`.
2. Tags `main` `releaseX.Y.Z`.
3. Merges `main` back into the release branch, which changes no files but means a later pull request only shows new
   changes, and deletes the test tag.
4. Creates a draft GitHub release for the tag, with the release's section of the ChangeLog as its notes.
5. Runs the [Release workflow](../.github/workflows/release.yml) on the tag, which builds and tests the installers and
   attaches them to the draft. The files are only attached if all four builds succeed.
6. Downloads them from the draft, all in one folder, and shows the draft's URL:

```
~/Downloads/bruce-smart-home-X.Y.Z/BruceSmartHome_X.Y.Z_aarch64.dmg      Apple silicon Mac
~/Downloads/bruce-smart-home-X.Y.Z/BruceSmartHome_X.Y.Z_x86_64.dmg       Intel Mac
~/Downloads/bruce-smart-home-X.Y.Z/BruceSmartHome_X.Y.Z_arm64-setup.exe  ARM PC
~/Downloads/bruce-smart-home-X.Y.Z/BruceSmartHome_X.Y.Z_x64-setup.exe    Intel or AMD PC
```

Then it stops. Try the installers, and check the release notes on GitHub (the draft is at the top of the Releases page).
A draft is only visible to people who can push to the repo.

If it stops part way, e.g. a build fails, or Ctrl+C stops it watching, run `release` again: it carries on where it
stopped, watching a run which is still going rather than starting another. A failed run's jobs can be re-run with the
commands it shows, then `bruce-method-release watch` picks it up again.

To make more changes before the release is published:

```
bruce-method-release untag
```

deletes the draft release and the release tag. Commit the changes to the release branch, `try` them, and run `release`
again, which merges the new changes.

### Release Notes

The notes are the `## Release X.Y.Z` section of [ChangeLog.md](ChangeLog.md) as it is at the `releaseX.Y.Z` tag,
without that heading, as the release has its own title, `bruce-smart-home X.Y.Z`. So write the ChangeLog for the
release before `release`, which checks that it has a section for the release. GitHub shows every line break in release
notes, so the lines of each paragraph, list item and quote are joined, and links to other docs, e.g.
`[...](ScheduledFetch.md)`, are made links to the docs on GitHub at the tag.

To change the notes of a draft, edit them on GitHub (choose **Edit** on the draft), or change the ChangeLog: `untag`,
commit, `try`, `release`.

<details>
<summary>Manual commands</summary>

```
cd ~/git/bruce-smart-home
git switch v$RELEASE
git status --short
git push
gh pr create --base main --head v$RELEASE --title "Release $RELEASE" --body "Release $RELEASE"
gh pr merge v$RELEASE --squash
```

The pull request can be merged on GitHub instead, choose **Squash and merge**. Then tag `main`, merge it back into
the release branch, make the draft and build the installers:

```
git fetch origin
git tag --annotate release$RELEASE origin/main --message "bruce-smart-home release$RELEASE"
git push origin release$RELEASE
git merge origin/main -m "Merge main after squash merge"
git push
gh release create release$RELEASE --draft --verify-tag --title "bruce-smart-home $RELEASE" --notes-file <file>
gh workflow run release.yml --ref release$RELEASE -f tag=release$RELEASE -f attach=true
```

`<file>` is the release's section of the ChangeLog, without its heading, with the lines of each paragraph joined, see
[Release Notes](#release-notes). `--generate-notes` in place of `--notes-file <file>` lists the merged pull requests
instead. Wait a few seconds for the run to start, then watch it and download the installers from the draft:

```
RUN=$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch $RUN
gh release download release$RELEASE --dir ~/Downloads/bruce-smart-home-$RELEASE
```

On a Windows VM in PowerShell, just the Windows installers (with the version in place of 0.6.0):

```
gh release download release0.6.0 -R bruceskingle/bruce-smart-home -p '*-setup.exe' --dir $HOME\Downloads\bruce-smart-home
```

To make more changes, delete the draft release and the release tag:

```
gh release delete release$RELEASE --yes
git tag -d release$RELEASE
git push origin --delete release$RELEASE
```

</details>

### Capture Firmware

Attach each board's capture firmware to the draft, so that the app's Devices page, and anything else which flashes
these boards, can download it, see `bruce-embedded-std.md` and bruce-embedded-flash's `bruce-embedded-firmware`.
Build it from the tagged commit, which puts it in the shared firmware folder as `bruce-smart-home/dev`, then upload
it:

```
cd ~/git/bruce-smart-home/capture
./build-firmware.sh
gh release upload release$RELEASE "$(dirname "$(./build-firmware.sh --path cyd-esp32-28)")"/*.elf
```

The repository needs the GitHub topic `bruce-embedded-firmware` for the apps to find it, once.

## 3. Publish

```
bruce-method-release release
```

again, once the installers have been tried, checks that the draft has all four installers, then publishes it as the
latest release. Nothing is built again, so the installers are the ones which were tried. Then it starts the next
release: it creates the `vX.Y.Z` branch for the next minor version (e.g. 0.7.0 after 0.6.0) from `main`, sets the
version in Cargo.toml, updates Cargo.lock, commits and pushes.

Then commit to the new branch and push as usual. When it is ready to release, go back to step 1.

<details>
<summary>Manual commands</summary>

```
gh release edit release$RELEASE --draft=false --latest
```

or choose **Publish release** on the draft's edit page on GitHub. Then start the next version with
`bruce-method-release start`.

</details>

### Patch Releases

Patch versions are kept for fixes to a release, so that a fix can be released without the new features being developed
for the next one. Start one from the release's tag, e.g. while 0.7.0 is being developed:

```
bruce-method-release start 0.6.1
```

Make the fix and release it as usual. Then merge `main` into the release branch for the next minor version, so that it
has the fix too, keeping its own version where Cargo.toml conflicts:

```
bruce-method-release sync 0.7.0
git merge origin/main
```

If a change needs its own branch, e.g. because someone else is working on it, branch from the release branch and
merge it back into it, or merge their pull request on GitHub with the release branch as the base:

```
git switch -c my-feature v0.6.0
# ... commit and push my-feature, then when it is done
git switch v0.6.0
git pull
git merge my-feature
git push
```

## Other Build and Test Machines

To build and test the latest release branch on another machine:

```
bruce-method-release sync
```

Fetches the repo and updates the branch it is on, moving on to the latest `vX.Y.Z` branch if its version has been
released. For another branch, give it, e.g. `bruce-method-release sync 0.6.0` or `bruce-method-release sync main`. It
stops without changing anything if the repo has uncommitted changes, and only fast forwards, so it stops if the branch
has commits on that machine which are not on GitHub. Then build as usual, e.g. `dx serve`.

<details>
<summary>Manual commands</summary>

```
cd ~/git/bruce-smart-home
git fetch
git switch v$RELEASE
git pull --ff-only
```

</details>

## Trying the Installer on Another Machine

When `try` or `release` has built the installers on the main dev machine, on the other machine run:

```
bruce-method-release sync
bruce-method-release download
```

`sync` puts the repo on the release branch, which `download` needs because the version comes from it. `download`
then downloads just the installer for that machine, from the latest run: the `.dmg` for an Apple silicon or Intel Mac,
or the `setup.exe` for an ARM or Intel/AMD PC. It stops if the run has not finished, so wait for it with `watch` first.

After `try`, the installer is in a folder named after its build, e.g. on an Apple silicon Mac:

```
~/Downloads/bruce-smart-home-X.Y.Z-test/macos-aarch64/BruceSmartHome_X.Y.Z_aarch64.dmg
```

After `release`, it is downloaded from the draft release, e.g. on an ARM PC:

```
~/Downloads/bruce-smart-home-X.Y.Z/BruceSmartHome_X.Y.Z_arm64-setup.exe
```

The folder and file names for each machine are listed in [step 1](#1-try-the-installers) and
[step 2](#2-release-up-to-the-draft). On Windows `~` is the user's folder, e.g. `C:\Users\yourname`.

On Windows, in PowerShell (or a Command Prompt), one command does both:

```
~\git\bruce-smart-home\scripts\sync-and-download.cmd
```

It needs bruce-method-release, installed with `cargo install bruce-method-release` (which needs Rust, from
https://rustup.rs), and `gh` installed and signed in, as in the manual commands of [step 1](#1-try-the-installers).

# One Time Setup Steps

## Signing the macOS App

The release workflow signs the macOS app and its `.dmg`, and has Apple notarize them, so that a Mac opens them without
warning that the developer can not be verified. `dx bundle` does it all, given five secrets in the GitHub repository:
a **Developer ID Application certificate**, which says the app is from you, and an **App Store Connect API key**, with
which the workflow asks Apple to notarize it. Without the secrets the app is built unsigned, as before. The steps are
"Set up signing and notarizing" and "Check the app is signed and notarized" in
[release.yml](../.github/workflows/release.yml).

This is done once, then again only when the certificate or the key expires or is replaced. It needs the Apple
Developer account's Account Holder or an Admin, and is done on the Mac the release is run from, in Terminal, in
`~/git/bruce-smart-home`.

### Part 1: the certificate

1. **Check whether you already have one.** In Terminal:

   ```
   security find-identity -v -p codesigning
   ```

   If it lists `Developer ID Application: YOUR NAME (TEAM ID)`, skip to step 7. "0 valid identities
   found" means carry on. (A **Developer ID Installer** certificate does not count: it only signs `.pkg` installers,
   and is not listed here.)
2. **Make a certificate request.** Open **Keychain Access** (Spotlight finds it). From its menu: Keychain Access >
   Certificate Assistant > **Request a Certificate From a Certificate Authority…**. User Email Address: your Apple
   developer email. Common Name: e.g. `Bruce's Smart Home Developer ID`. CA Email Address: empty. Choose **Saved to disk**,
   Continue, and save the `.certSigningRequest` file, e.g. to the Desktop. This also makes the certificate's private
   key in your login keychain, so do it on this Mac.
3. **Ask Apple for the certificate.** Go to [Certificates](https://developer.apple.com/account/resources/certificates/list)
   (developer.apple.com > Account > Certificates, IDs & Profiles > Certificates), click **+**, choose
   **Developer ID Application** (not Installer), Continue, upload the `.certSigningRequest` file, and download the
   certificate, a `.cer` file.
4. **Install it.** Open the `.cer` file, which adds it to the login keychain.
5. **If Keychain Access says it is not trusted**, the Mac is missing Apple's intermediate certificate, which Xcode
   would have installed: download **Developer ID - G2** from [Apple's certificate authority
   page](https://www.apple.com/certificateauthority/) and open it.
6. **Check** with the command in step 1, which should now list `Developer ID Application: ...`.
7. **Export it as a `.p12` file.** In Keychain Access, login keychain, **My Certificates**, find
   `Developer ID Application: YOUR NAME (TEAM ID)`, which has its private key under it. Right-click >
   Export, format Personal Information Exchange (`.p12`), e.g. `~/Desktop/DeveloperID.p12`, and give it a password,
   which you will need in step 14.

### Part 2: the API key

8. **Open the keys page.** Go to [App Store Connect > Users and Access > Integrations](https://appstoreconnect.apple.com/access/integrations/api)
   and choose **App Store Connect API**, then the **Team Keys** tab. (Not Individual Keys, which have no Issuer ID.)
9. **Note the Issuer ID.** It is above the list of keys, labelled Issuer ID, a UUID, five groups of letters and digits
   joined by hyphens, e.g. `69a6de7e-1234-47e3-e053-5b8c7c11a4d1`. Copy it with the Copy link next to it.
10. **Make a key, or use the one there.** If there is no key yet, click **+** (Generate API Key), name it e.g.
    `Bruce's Smart Home notarize`, Access **Developer**, and Generate.
11. **Note the Key ID.** It is in the key's row, in the KEY ID column, 10 capital letters and digits, e.g. `2X9R4HXF34`.
12. **Download the key**, from the key's row, a file named `AuthKey_` and its Key ID, e.g. `AuthKey_2X9R4HXF34.p8`,
    which goes to Downloads. Apple lets it be downloaded only once: if it was downloaded before and lost, revoke that
    key and make a new one, step 10.
13. **Check the key with Apple**, on your Mac, putting in your file, Key ID and Issuer ID:

    ```
    xcrun notarytool history --key ~/Downloads/AuthKey_2X9R4HXF34.p8 --key-id 2X9R4HXF34 --issuer 69a6de7e-1234-47e3-e053-5b8c7c11a4d1
    ```

    It lists the notarizations so far, none the first time, or says what is wrong, e.g. that the issuer is not a UUID.

### Part 3: the secrets on GitHub

14. **Set the five secrets**, in Terminal in `~/git/bruce-smart-home`, putting in your own file names, Key ID and Issuer ID
    as in step 13. The files are read and sent as they are, so nothing secret is typed or shown:

    ```
    base64 -i ~/Desktop/DeveloperID.p12 | gh secret set APPLE_CERTIFICATE
    gh secret set APPLE_CERTIFICATE_PASSWORD
    gh secret set APPLE_API_KEY --body 2X9R4HXF34
    gh secret set APPLE_API_ISSUER --body 69a6de7e-1234-47e3-e053-5b8c7c11a4d1
    base64 -i ~/Downloads/AuthKey_2X9R4HXF34.p8 | gh secret set APPLE_API_KEY_CONTENT
    ```

    The second asks for the `.p12`'s password, from step 7.
15. **Check they are there**, by name, as GitHub never shows a secret again:

    ```
    gh secret list
    ```

    It should list the five `APPLE_...` secrets, with when each was set.
16. **Put the files somewhere safe**, e.g. a password manager, with the `.p12`'s password, and delete them from the
    Desktop and Downloads.

### Part 4: try it

17. **Run a try**, see [Try the Installers](#1-try-the-installers-optional): `./scripts/release.sh try`.
18. **Watch the Mac builds' logs** on GitHub (Actions > the Release run > Build macOS Apple Silicon, or Intel):
    - "Set up signing and notarizing" should not say "No APPLE_CERTIFICATE secret".
    - "Build the installers" takes a few minutes longer, while Apple notarizes, and should say the notarization was
      Accepted and stapled.
    - "Check the app is signed and notarized" should show `Authority=Developer ID Application: ...`, and `accepted`,
      `source=Notarized Developer ID`, for both the app and the `.dmg`.
19. **Open the downloaded `.dmg` on a Mac** and start the app: it should open with no warning that it can not be
    verified, and no need to right-click > Open.

If notarizing fails, "Build the installers" fails, with Apple's reason. `dx bundle` does not ask `codesign` for a secure
timestamp, which notarization needs; if Apple says "The signature does not include a secure timestamp", the app will
need signing again with `--timestamp` after `dx bundle`.
