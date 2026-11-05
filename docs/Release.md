# marco-sparko
# Release Process

Each release is developed on a branch called `vX.X.X` in both sparko_graphql and marco-sparko, which is squash
merged to `main` and tagged `releaseX.X.X`. The installers are built on GitHub and attached to a draft release, which
is published once they have been tried. Then the branches for the next version are started.

The steps are run with [scripts/release.sh](../scripts/release.sh) on the main dev machine, with both repos checked
out side by side in `~/git`. It needs the GitHub CLI, `gh`, signed in with `gh auth login`. The version comes from
marco-sparko's Cargo.toml, which `start` sets at the beginning of each release, so it never needs to be typed again.
Each command checks that both repos are in the state it expects (the right branch, nothing uncommitted, the tags it
needs) and stops with an explanation at the first problem.

```
cd ~/git/marco-sparko
./scripts/release.sh status
```

shows where things are: the branch, version and uncommitted changes of each repo, the test and release tags, the
GitHub release and the latest workflow runs.

The manual commands for each step are in the collapsed sections, in case the script cannot be used. They need the
version in `RELEASE`, e.g. `export RELEASE=0.4.4`.

## 1. Try the Installers (optional)

```
./scripts/release.sh try
```

Builds the installers from the release branches before anything is merged, to try them on other machines. It tags both
release branches `testX.X.X` (moving the tags if they exist), runs the workflow, watches it (about 30 to 40 minutes,
the Intel Mac is the slowest) and downloads the installers, each in a folder named after its build:

```
~/Downloads/marco-sparko-X.X.X-test/macos-aarch64/MarcoSparko_X.X.X_aarch64.dmg      Apple silicon Mac
~/Downloads/marco-sparko-X.X.X-test/macos-x64/MarcoSparko_X.X.X_x86_64.dmg           Intel Mac
~/Downloads/marco-sparko-X.X.X-test/windows-arm64/MarcoSparko_X.X.X_arm64-setup.exe  ARM PC
~/Downloads/marco-sparko-X.X.X-test/windows-x64/MarcoSparko_X.X.X_x64-setup.exe      Intel or AMD PC
```

Each run of `try` replaces them. Run it again after fixing anything. The test tags are deleted by `merge`.

If a build fails, it shows the commands to see why and to re-run the failed jobs.

Ctrl+C stops watching but not the run. To pick it up again, e.g. after a re-run of failed jobs, and download the
installers when it finishes:

```
./scripts/release.sh watch
```

To try the installer on another machine, see
[Trying the Installer on Another Machine](#trying-the-installer-on-another-machine).

<details>
<summary>Manual commands</summary>

```
for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git tag -f test$RELEASE
  git push -f origin test$RELEASE
done

cd ~/git/marco-sparko
gh workflow run release.yml --ref v$RELEASE -f tag=test$RELEASE
```

`--ref` runs the version of the workflow on the release branch, so changes to it can be tried before they are merged.
Wait a few seconds for the run to start, then watch it and download the installers:

```
RUN=$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch $RUN
gh run download $RUN --dir ~/Downloads/marco-sparko-$RELEASE-test
```

Each build's installers are kept as an artifact named `macos-aarch64`, `macos-x64`, `windows-x64` or `windows-arm64`
for 90 days. Particular builds can be downloaded with `-n`, e.g. while the slower builds are still running (if `gh`
refuses because the run is in progress, wait for it to finish):

```
gh run download $RUN -n windows-arm64 -n macos-aarch64 --dir ~/Downloads/marco-sparko-$RELEASE-test
gh api repos/bruceskingle/marco-sparko/actions/runs/$RUN/artifacts --jq '.artifacts[] | "\(.name) \(.size_in_bytes)"'
```

On a Windows VM, in PowerShell:

1. Install `gh` with `winget install --id GitHub.cli`, or from https://github.com/cli/cli/releases (the "windows
   arm64 installer" or "windows amd64 installer", which is the `.msi`). Then open a new PowerShell window and sign in
   with `gh auth login`.

2. Find the run ID, the number which identifies the run of the workflow. It is shown in any of these places:
   - On the Mac, in the output of `try`, `build` or `watch`: `==> Watching run 40215599738 ...`
   - In the first line which `gh run watch` shows, after "Release ·": `* v0.4.4 Release · 40215599738`. The other
     numbers it shows, `(ID 121252356794)` after each job, are job IDs, which are not what is needed here (they are
     only used to get one job's log, see below).
   - In the address of the run's page on the Actions tab: `.../marco-sparko/actions/runs/40215599738`
   - Listed with the recent runs, on either machine, with
     `gh run list -R bruceskingle/marco-sparko --workflow release.yml --limit 5`
     (the ID is the long number in the ID column, the newest run is first).

3. Set `$RUN` to it. The variables set on the Mac are not set in PowerShell, so this is always needed there. With the
   number from step 2 in place of 40215599738:

   ```
   $RUN = "40215599738"
   ```

   or, to use the latest run without looking up its number:

   ```
   $RUN = gh run list -R bruceskingle/marco-sparko --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId'
   ```

   `echo $RUN` shows which run it is set to.

4. Download the installer, `windows-arm64` on an ARM PC or `windows-x64` on an Intel or AMD PC:

   ```
   gh run download $RUN -R bruceskingle/marco-sparko -n windows-arm64 --dir $HOME\Downloads\marco-sparko-test
   ```

   `-R` means these commands work in any directory. The setup.exe is in `Downloads\marco-sparko-test`.

The artifacts can also be downloaded from the run's page on the Actions tab of the marco-sparko repo, under
**Artifacts** at the bottom of the Summary (only the name is a link).

If a build fails, see why with the first command, or the second if that prints nothing (the job ID is shown next to
the job by `gh run watch`), and re-run the failed jobs with the third:

```
gh run view $RUN --log-failed
gh api repos/bruceskingle/marco-sparko/actions/jobs/<job id>/logs | grep -B2 -A10 -i "error\|panicked"
gh run rerun $RUN --failed
```

Delete the test tags afterwards:

```
for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git tag -d test$RELEASE
  git push origin --delete test$RELEASE
done
```

</details>

## 2. Merge to Main and Tag

```
./scripts/release.sh merge
```

For each repo, sparko_graphql first: creates a pull request from the release branch (unless there is one), squash
merges it to `main` (`gh` asks for confirmation), and tags `main` `releaseX.X.X`. It then merges `main` back into the
release branch, which changes no files but means a later pull request only shows new changes, and deletes the test
tags. If it fails part way, run it again and it carries on.

To make more changes after merging, before the release is published:

```
./scripts/release.sh untag
```

deletes the release tags (and the draft release, if `build` has made one). Commit the changes to the release
branches, then run `merge` again.

<details>
<summary>Manual commands</summary>

```
for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git checkout v$RELEASE
  git status --short
  git push
  gh pr create --base main --head v$RELEASE --title "V$RELEASE" --body "Release $RELEASE"
  gh pr merge v$RELEASE --squash
done
```

The pull requests can be merged on GitHub instead, choose **Squash and merge**. Then tag `main` and merge it back
into the release branches:

```
for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git fetch origin
  git tag release$RELEASE origin/main
  git push origin release$RELEASE
  git merge origin/main -m "Merge main after squash merge"
  git push
done
```

To make more changes, delete the draft release (if there is one) and the release tags:

```
gh release delete release$RELEASE --yes
for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git tag -d release$RELEASE
  git push origin --delete release$RELEASE
done
```

</details>

## 3. Final Check

```
./scripts/release.sh check
```

Checks out the release tags in both repos, does a clean build and runs the app. Click around, then quit the app and it
switches both repos back to the release branches. If something needs fixing, use `untag` as in step 2.

<details>
<summary>Manual commands</summary>

```
cd ~/git/sparko_graphql
git checkout release$RELEASE
cd ~/git/marco-sparko
git checkout release$RELEASE
cargo clean
dx run --release
```

Then go back to the release branches. If Cargo.lock shows as changed, it was regenerated (e.g. by the editor) and it
is safe to discard it:

```
for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git restore Cargo.lock
  git checkout v$RELEASE
done
```

</details>

## 4. Build the Draft Release

```
./scripts/release.sh build
```

Creates a draft GitHub release for the tag, with the release's section of the [ChangeLog](ChangeLog.md) as its notes
(see [Release Notes](#release-notes)), then runs the
[Release workflow](../.github/workflows/release.yml) on the tag, which builds and tests the installers and attaches
them to the draft. The files are only attached if all four builds succeed. It downloads them from the draft, all in
one folder, and shows the draft's URL:

```
~/Downloads/marco-sparko-X.X.X/MarcoSparko_X.X.X_aarch64.dmg      Apple silicon Mac
~/Downloads/marco-sparko-X.X.X/MarcoSparko_X.X.X_x86_64.dmg       Intel Mac
~/Downloads/marco-sparko-X.X.X/MarcoSparko_X.X.X_arm64-setup.exe  ARM PC
~/Downloads/marco-sparko-X.X.X/MarcoSparko_X.X.X_x64-setup.exe    Intel or AMD PC
```

Try the installers, and check the release notes on GitHub (the draft is at the top of the Releases page). A draft is
only visible to people who can push to the repo.

### Release Notes

The notes are the `## Release X.X.X` section of [ChangeLog.md](ChangeLog.md) as it is at the `releaseX.X.X` tag,
without that heading, as the release has its own title. So write the ChangeLog for the release before `merge`, and
`build` stops if it has no section for the release. GitHub shows every line break in release notes, so the lines of
each paragraph, list item and quote are joined, and links to other docs, e.g. `[...](ScheduledFetch.md)`, are made
links to the docs on GitHub at the tag.

Running `build` again on an existing draft leaves its notes alone. To change them, either edit them on GitHub (choose
**Edit** on the draft), or change the ChangeLog (`untag`, commit, `merge`, as in step 2) and replace the notes with
its section with:

```
./scripts/release.sh notes
```

If a build fails, fix it as in step 2 (`untag`, commit, `merge`, `check`) and run `build` again. Running `build` again
on an existing draft replaces its installers. As with `try`, `./scripts/release.sh watch` picks up the run again after
Ctrl+C or a re-run of failed jobs, and downloads the installers from the draft when it finishes. To try the installer
on another machine, see [Trying the Installer on Another Machine](#trying-the-installer-on-another-machine).

<details>
<summary>Manual commands</summary>

```
cd ~/git/marco-sparko
gh release create release$RELEASE --draft --verify-tag --title "Release $RELEASE" --notes-file <file>
gh workflow run release.yml --ref release$RELEASE -f tag=release$RELEASE -f attach=true
```

Wait a few seconds for the run to start, then watch it and download the installers from the draft:

```
RUN=$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch $RUN
gh release download release$RELEASE --dir ~/Downloads/marco-sparko-$RELEASE
```

`<file>` is the release's section of the ChangeLog, without its heading, with the lines of each paragraph joined, see
[Release Notes](#release-notes). `--generate-notes` in place of `--notes-file <file>` lists the merged pull requests
instead.

On a Windows VM in PowerShell, just the Windows installers (with the version in place of 0.4.4):

```
gh release download release0.4.4 -R bruceskingle/marco-sparko -p '*-setup.exe' --dir $HOME\Downloads\marco-sparko
```

The release should have these files:

```
MarcoSparko_X.X.X_aarch64.dmg
MarcoSparko_X.X.X_x86_64.dmg
MarcoSparko_X.X.X_x64-setup.exe
MarcoSparko_X.X.X_arm64-setup.exe
```

</details>

## 5. Publish

```
./scripts/release.sh publish
```

Checks that the draft has all four installers, then publishes it as the latest release. Nothing is built again, so the
installers are the ones which were tried.

<details>
<summary>Manual commands</summary>

```
gh release edit release$RELEASE --draft=false --latest
```

or choose **Publish release** on the draft's edit page on GitHub.

</details>

## 6. Start the Next Release

```
./scripts/release.sh start
```

Creates the `vX.X.X` branches for the next minor version (e.g. 0.5.0 after 0.4.4) in both repos from `main`, sets
the version in both Cargo.toml files and the version of sparko_graphql which marco-sparko uses, builds to update the
Cargo.lock files, commits and pushes. For a different version, give it, e.g. `./scripts/release.sh start 0.4.5`. If
the branches already exist it switches to them.

Then commit to the `vX.X.X` branches and push as usual. When it is ready to release, go back to step 1.

### Patch Releases

Patch versions are kept for fixes to the last release, so that a fix can be released without the new features being
developed for the next one. Until the next minor version is merged, `main` is the last release, so start the patch
release from it, e.g. while 0.5.0 is being developed:

```
./scripts/release.sh start 0.4.5
```

Make the fix and release it as usual. Then merge `main` into the release branch for the next minor version, so that it
has the fix too, keeping its own version where the Cargo.toml files conflict:

```
./scripts/release.sh sync v0.5.0
git -C ~/git/marco-sparko merge origin/main
git -C ~/git/sparko_graphql merge origin/main
```

If a change needs its own branch, e.g. because someone else is working on it, branch from the release branch and
merge it back into it, or merge their pull request on GitHub with the release branch as the base:

```
git checkout -b my-feature v0.4.4
# ... commit and push my-feature, then when it is done
git checkout v0.4.4
git pull
git merge my-feature
git push
```

<details>
<summary>Manual commands</summary>

```
export RELEASE=0.4.5

for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git checkout main
  git pull
  git checkout -b v$RELEASE
done

cd ~/git/sparko_graphql
sed -i '' "1,/^version = /s/^version = .*/version = \"$RELEASE\"/" Cargo.toml
cargo build

cd ~/git/marco-sparko
sed -i '' -e "1,/^version = /s/^version = .*/version = \"$RELEASE\"/" \
  -e "s/^sparko_graphql = {version = \"[^\"]*\"/sparko_graphql = {version = \"$RELEASE\"/" Cargo.toml
cargo build

for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git diff --stat
  git commit -am "Start v$RELEASE"
  git push -u origin v$RELEASE
done
```

</details>

## Other Build and Test Machines

To build and test the latest release branch on another machine, with both repos checked out side by side there too:

```
./scripts/release.sh sync
```

Fetches both repos and checks out and updates the latest `vX.X.X` branch in each, so that marco-sparko is built with
the matching sparko_graphql. For another branch, give it, e.g. `./scripts/release.sh sync 0.4.4` or
`./scripts/release.sh sync main`. It does not need `gh`. It stops without changing anything if either repo has
uncommitted changes, and only fast forwards, so it stops if a branch has commits on that machine which are not on
GitHub. Then build as usual, e.g. `dx serve`.

<details>
<summary>Manual commands</summary>

```
export RELEASE=0.4.4

for repo in sparko_graphql marco-sparko; do
  cd ~/git/$repo
  git fetch
  git checkout v$RELEASE
  git pull --ff-only
done
```

</details>

## Trying the Installer on Another Machine

When `try` or `build` has finished on the main dev machine, on the other machine run:

```
./scripts/release.sh sync
./scripts/release.sh download
```

`sync` puts the repos on the release branch, which `download` needs because the version comes from them. `download`
then downloads just the installer for that machine, from the latest run: the `.dmg` for an Apple silicon or Intel Mac,
or the `setup.exe` for an ARM or Intel/AMD PC. It stops if the run has not finished, so wait for it with `watch` first.

After `try`, the installer is in a folder named after its build, e.g. on an Apple silicon Mac:

```
~/Downloads/marco-sparko-X.X.X-test/macos-aarch64/MarcoSparko_X.X.X_aarch64.dmg
```

After `build`, it is downloaded from the draft release, e.g. on an ARM PC:

```
~/Downloads/marco-sparko-X.X.X/MarcoSparko_X.X.X_arm64-setup.exe
```

The folder and file names for each machine are listed in [step 1](#1-try-the-installers-optional) and
[step 4](#4-build-the-draft-release). On Windows `~` is the user's folder, e.g. `C:\Users\yourname`.

On Windows, in PowerShell (or a Command Prompt), one command does both:

```
~\git\marco-sparko\scripts\sync-and-download.cmd
```

It runs `release.sh` in Git Bash, which comes with Git for Windows. `gh` needs to be installed and signed in, as in the
manual commands of [step 1](#1-try-the-installers-optional). The other commands of `release.sh` are for the Mac.

Git for Windows usually gives files Windows line endings, which bash cannot run, so
[.gitattributes](../.gitattributes) keeps `release.sh` with Unix ones. A copy checked out before that was added may
still have Windows ones, and fail with errors like `$'\r': command not found`. Fix it once, in PowerShell:

```
cd ~\git\marco-sparko
git pull
Remove-Item scripts\release.sh
git checkout scripts/release.sh
```

# Old Release Instructions
The manual build steps used before the workflow are in [Release-archive.md](Release-archive.md).

## Signing the macOS App

The workflow signs the macOS app and its `.dmg` with a Developer ID certificate, and notarizes both with Apple, so
that a Mac opens them without warning that the developer can not be verified. `dx bundle` does both, given the
certificate and an App Store Connect API key in the repository's secrets, see "Set up signing and notarizing" in
[release.yml](../.github/workflows/release.yml). Without the secrets the app is built unsigned, as before. "Check the app
is signed and notarized" in each Mac build's log says how it went: who signed it, and that Gatekeeper accepts it.

Setting it up, once, needs the Apple Developer account's Account Holder or an Admin:

1. **The certificate.** In Keychain Access, Certificate Assistant > Request a Certificate From a Certificate
   Authority, with your email address, saved to disk. At
   [Certificates](https://developer.apple.com/account/resources/certificates/list), + > **Developer ID Application**,
   upload that request, and download the certificate. Open it, which adds it to the login keychain. In Keychain
   Access, under My Certificates, export "Developer ID Application: *your name* (*team ID*)", with its private key,
   as a `.p12` file, with a password.
2. **The API key.** In [App Store Connect](https://appstoreconnect.apple.com/access/integrations/api), Users and Access
   > Integrations > App Store Connect API > Team Keys, generate a key with the **Developer** role. Download its
   `AuthKey_KEYID.p8`, which can only be downloaded once, and note its Key ID and the Issuer ID above the list.
3. **The secrets**, in this repo's directory, from the files, so that nothing is typed or shown:

   ```
   base64 -i DeveloperID.p12 | gh secret set APPLE_CERTIFICATE
   gh secret set APPLE_CERTIFICATE_PASSWORD                  # asks for the .p12's password
   gh secret set APPLE_API_KEY --body KEYID
   gh secret set APPLE_API_ISSUER --body ISSUER-ID
   base64 -i AuthKey_KEYID.p8 | gh secret set APPLE_API_KEY_CONTENT
   ```

4. Keep the `.p12` and `.p8` somewhere safe, e.g. a password manager, and delete the copies on disk. A new key or
   certificate only needs its secrets set again.

Then [try the installers](#1-try-the-installers-optional). `dx bundle` does not ask `codesign` for a secure timestamp,
which notarization needs; if the notarization fails for that, the log says "The signature does not include a secure
timestamp", and the app will need signing again with `--timestamp` after `dx bundle`.
