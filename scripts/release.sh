#!/bin/bash
# The release process for marco-sparko and bruce-graphql, see docs/Release.md
#
# The version comes from the version in marco-sparko's Cargo.toml, which "start" sets at the beginning of each release
# cycle. Each command checks that the repos are in the state it expects before changing anything, and stops at the
# first problem.
#
# This is for macOS (it uses BSD sed) with both repos checked out side by side, e.g. ~/git/marco-sparko and
# ~/git/bruce-graphql, and needs the GitHub CLI (gh) signed in. sync and download also work in Git Bash on Windows, to
# test there.

set -euo pipefail

# bruce-graphql first, as marco-sparko depends on it.
REPOS="bruce-graphql marco-sparko"
WORKFLOW=release.yml
# The installers attached to each release, with VERSION replaced by the version. The Intel Mac's was _x64.dmg before
# dx 0.7.10.
INSTALLERS="MarcoSparko_VERSION_aarch64.dmg MarcoSparko_VERSION_x86_64.dmg MarcoSparko_VERSION_x64-setup.exe
MarcoSparko_VERSION_arm64-setup.exe"

GIT_ROOT=$(cd "$(dirname "$0")/../.." && pwd)

# The GitHub account or organisation of the repos, from marco-sparko's origin, so that the script also works on forks
# (e.g. to try it out), checked out side by side in another directory.
github_owner() {
    git -C "$GIT_ROOT/$1" remote get-url origin | sed -E 's#^.*github\.com[:/]([^/]+)/.*$#\1#'
}
OWNER=$(github_owner marco-sparko)
[ "$(github_owner bruce-graphql)" = "$OWNER" ] \
    || { echo "ERROR: marco-sparko is from $OWNER but bruce-graphql is from $(github_owner bruce-graphql)" >&2; exit 1; }

usage() {
    cat <<EOF
Usage: $(basename "$0") <command>

  status           Show the state of both repos, the tags and the GitHub release
  start [version]  Start the next release: create the vX.X.X branches from main and set the version
                   (default: the next minor version after the one on main, e.g. 0.5.0 after 0.4.4; give a patch
                   version, e.g. 0.4.5, for a fix to the last release)
  try              Build the installers from the release branches, without merging, and download them
  merge            Squash merge the release branches to main, tag them releaseX.X.X, and merge main back
  check            Clean build and run the tagged source, then go back to the release branches
  build            Create a draft GitHub release, with the notes from the ChangeLog, build and attach the installers,
                   and download them
  notes            Replace the draft release's notes with the release's section of the ChangeLog, as it is at the tag
  publish          Publish the draft release, once the installers have been tried and the notes edited
  untag            Delete the release tags (and draft release) to make more changes before releasing
  watch            Watch the latest run of try or build again, e.g. after Ctrl+C, and download its installers
  download         Download the installer for this machine from the latest try or build, e.g. on another machine
  sync [branch]    On another machine, e.g. to build and test there: fetch both repos and check out and update the
                   same branch in each, e.g. 0.4.4, v0.4.4 or main (default: the latest vX.X.X release branch)
EOF
    exit 1
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

info() {
    echo "==> $*"
}

repo_dir() {
    echo "$GIT_ROOT/$1"
}

# The package version in the Cargo.toml in the given directory.
cargo_version() {
    sed -n 's/^version = "\(.*\)"/\1/p' "$1/Cargo.toml" | head -1
}

# The version being released, from marco-sparko's Cargo.toml, after checking that bruce-graphql has the same version.
release_version() {
    local version graphql_version dependency_version
    version=$(cargo_version "$(repo_dir marco-sparko)")
    graphql_version=$(cargo_version "$(repo_dir bruce-graphql)")
    dependency_version=$(sed -n 's/^bruce-graphql = {version = "\([^"]*\)".*/\1/p' "$(repo_dir marco-sparko)/Cargo.toml")
    [ -n "$version" ] || die "No version in marco-sparko/Cargo.toml"
    [ "$graphql_version" = "$version" ] || die "bruce-graphql is version $graphql_version but marco-sparko is $version"
    [ "$dependency_version" = "$version" ] || die "marco-sparko depends on bruce-graphql $dependency_version but is version $version"
    echo "$version"
}

current_branch() {
    git -C "$(repo_dir "$1")" symbolic-ref --short -q HEAD || echo "(detached at $(git -C "$(repo_dir "$1")" describe --tags --always))"
}

require_gh() {
    gh auth status >/dev/null 2>&1 || die "gh is not signed in, run: gh auth login"
}

require_clean() {
    local dir
    dir=$(repo_dir "$1")
    if [ -n "$(git -C "$dir" status --porcelain)" ]; then
        git -C "$dir" status --short >&2
        die "$1 has uncommitted changes (if only Cargo.lock has changed, it is safe to run: git -C $dir restore Cargo.lock)"
    fi
}

# The given repo is on the release branch for the version in the Cargo.toml files, e.g. v0.4.4 for version 0.4.4.
require_branch() {
    local branch
    branch=$(current_branch "$1")
    if [ "$branch" != "$2" ]; then
        if [[ "$branch" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            die "$1 is on the release branch $branch, but the version in the Cargo.toml files is ${2#v}, so the release branch should be $2.
       If you are releasing ${branch#v}, set the version with: ./scripts/release.sh start ${branch#v}
       If you are releasing ${2#v}, switch branch with: git -C $(repo_dir "$1") checkout $2"
        fi
        die "$1 is on the branch $branch, but releasing version ${2#v} (from the Cargo.toml files) needs the release branch $2.
       Switch branch with: git -C $(repo_dir "$1") checkout $2"
    fi
}

require_pushed() {
    local dir
    dir=$(repo_dir "$1")
    git -C "$dir" fetch -q origin
    [ "$(git -C "$dir" rev-parse HEAD)" = "$(git -C "$dir" rev-parse '@{u}' 2>/dev/null)" ] \
        || die "$1 has commits which are not pushed, or is behind origin (git push / git pull)"
}

remote_tag_exists() {
    [ -n "$(git -C "$(repo_dir "$1")" ls-remote --tags origin "refs/tags/$2")" ]
}

# Deletes a tag locally and on GitHub, if it exists.
delete_tag() {
    local dir
    dir=$(repo_dir "$1")
    git -C "$dir" tag -d "$2" >/dev/null 2>&1 || true
    if remote_tag_exists "$1" "$2"; then
        git -C "$dir" push -q origin --delete "$2"
    fi
}

# Whether the GitHub release for the tag is a draft, a published release, or does not exist.
release_state() {
    local draft
    if draft=$(gh release view "$1" -R "$OWNER/marco-sparko" --json isDraft --jq .isDraft 2>/dev/null); then
        if [ "$draft" = "true" ]; then echo draft; else echo published; fi
    else
        echo none
    fi
}

latest_run_id() {
    gh run list -R "$OWNER/marco-sparko" --workflow "$WORKFLOW" --limit 1 --json databaseId --jq '.[0].databaseId // ""'
}

# Runs the workflow from ref to build tag, attaching the installers to the draft release for tag if attach is true,
# and waits for it to finish.
run_workflow() {
    local ref=$1 tag=$2 attach=$3 previous id
    previous=$(latest_run_id)
    info "Starting the $WORKFLOW workflow for $tag"
    gh workflow run "$WORKFLOW" -R "$OWNER/marco-sparko" --ref "$ref" -f tag="$tag" -f attach="$attach"

    id=$previous
    for _ in $(seq 30); do
        sleep 2
        id=$(latest_run_id)
        [ "$id" != "$previous" ] && break
    done
    [ "$id" != "$previous" ] || die "The workflow run did not start, see the Actions tab of the marco-sparko repo"

    watch_run "$id"
    RUN_ID=$id
}

# Waits for the workflow run to finish, and stops if it failed.
watch_run() {
    local id=$1
    info "Watching run $id (about 30 to 40 minutes), Ctrl+C stops watching but not the run, $(basename "$0") watch resumes"
    if ! gh run watch "$id" -R "$OWNER/marco-sparko" --exit-status; then
        echo >&2
        echo "To see why it failed: gh run view $id -R $OWNER/marco-sparko --log-failed" >&2
        echo "To re-run the failed jobs: gh run rerun $id -R $OWNER/marco-sparko --failed" >&2
        echo "To download the installers which were built, e.g. to try one which failed its check:" >&2
        echo "  gh run download $id -R $OWNER/marco-sparko --dir ~/Downloads/marco-sparko-run-$id" >&2
        die "The workflow run failed"
    fi
}

# Downloads the installers built by try's workflow run.
download_test_installers() {
    local version=$1 id=$2 download
    download="$HOME/Downloads/marco-sparko-$version-test"
    rm -rf "$download"
    gh run download "$id" -R "$OWNER/marco-sparko" --dir "$download"
    info "The installers are in $download"
}

# The release's section of docs/ChangeLog.md at the release tag, for the GitHub release notes, without its heading (the
# release has its own title). Links to other docs are relative, so they are made links to the docs on GitHub at the tag.
changelog_notes() {
    local tag="release$1"
    git -C "$(repo_dir marco-sparko)" show "$tag:docs/ChangeLog.md" \
        | awk -v version="$1" '/^## Release /{ show = ($3 == version); next } show' \
        | unwrap_markdown \
        | sed -E "s#\]\(([^)#:][^):]*)\)#](https://github.com/$OWNER/marco-sparko/blob/$tag/docs/\1)#g"
}

# Joins the lines of each paragraph, list item and quote, which the docs wrap at 120 characters, because GitHub shows
# each line break in release notes. Headings, list items, blank lines and code blocks start a new line.
unwrap_markdown() {
    awk '
        function flush() { if (holding) print held; held = ""; holding = 0 }
        function hold(line) { flush(); held = line; holding = 1 }
        /^[[:space:]]*```/ { flush(); code = !code; print; next }
        code { print; next }
        /^[[:space:]]*$/ || /^#/ || /^[[:space:]]*([-*+]|[0-9]+\.)[[:space:]]/ || /^[[:space:]]*\|/ { hold($0); next }
        /^>/ {
            text = $0
            sub(/^>[[:space:]]?/, "", text)
            if (held ~ /^>/ && text != "" && held !~ /^>[[:space:]]*$/) { held = held " " text } else { hold($0) }
            next
        }
        {
            text = $0
            sub(/^[[:space:]]+/, "", text)
            if (holding && held !~ /^[[:space:]]*$/ && held !~ /^#/ && held !~ /^[[:space:]]*\|/) { held = held " " text }
            else { hold($0) }
        }
        END { flush() }
    '
}

# Writes the release notes to a temporary file and prints its name, stopping if the ChangeLog has none.
changelog_notes_file() {
    local file
    file=$(mktemp)
    changelog_notes "$1" > "$file"
    grep -q '[^[:space:]]' "$file" \
        || die "docs/ChangeLog.md at release$1 has no \"## Release $1\" section for the release notes"
    echo "$file"
}

# Downloads the installers attached to the draft release by build's workflow run.
download_release_installers() {
    local version=$1 tag download
    tag="release$version"
    download="$HOME/Downloads/marco-sparko-$version"
    rm -rf "$download"
    gh release download "$tag" -R "$OWNER/marco-sparko" --dir "$download"
    info "The installers are in $download"
    echo
    echo "Try them, and check the release notes, from the ChangeLog, on GitHub (the draft is listed on the Releases page):"
    gh release view "$tag" -R "$OWNER/marco-sparko" --json url --jq .url
    echo
    info "When it is ready, next: $(basename "$0") publish"
}

# The name of the workflow's artifact with the installer for this machine. The architecture is the machine's own, not
# that of the shell, which may be running under Rosetta or Windows' x64 emulation.
this_platform() {
    case $(uname -s) in
        Darwin)
            if [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ]; then echo macos-aarch64; else echo macos-x64; fi
            ;;
        MINGW* | MSYS* | CYGWIN*)
            # Git Bash would otherwise turn /v into a path.
            if MSYS2_ARG_CONV_EXCL='*' reg query 'HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment' /v PROCESSOR_ARCHITECTURE 2>/dev/null \
                | grep -q ARM64; then
                echo windows-arm64
            else
                echo windows-x64
            fi
            ;;
        *) die "There are no installers for $(uname -s)" ;;
    esac
}

# The release's installer in the workflow's artifact, see INSTALLERS.
installer_pattern() {
    case $1 in
        macos-aarch64) echo '*_aarch64.dmg' ;;
        macos-x64) echo '*_x86_64.dmg' ;;
        windows-arm64) echo '*_arm64-setup.exe' ;;
        windows-x64) echo '*_x64-setup.exe' ;;
    esac
}

# The latest vX.X.X release branch on GitHub, by version, empty if there is none.
latest_release_branch() {
    git -C "$(repo_dir marco-sparko)" ls-remote --heads origin 'v*' \
        | sed -n 's#.*refs/heads/\(v[0-9]*\.[0-9]*\.[0-9]*\)$#\1#p' \
        | sort -t. -k1.2,1n -k2,2n -k3,3n | tail -1
}

cmd_sync() {
    local branch=${1:-}
    # A version on its own is the release branch for it.
    if [[ "$branch" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        branch="v$branch"
    fi
    if [ -z "$branch" ]; then
        branch=$(latest_release_branch)
        [ -n "$branch" ] || die "There is no vX.X.X release branch on GitHub, give the branch, e.g. main"
    fi
    # Checked before changing either repo.
    for repo in $REPOS; do
        require_clean "$repo"
        info "$repo: fetching"
        git -C "$(repo_dir "$repo")" fetch -q --prune origin
        git -C "$(repo_dir "$repo")" rev-parse -q --verify "refs/remotes/origin/$branch" >/dev/null \
            || die "$repo has no branch $branch on GitHub"
    done

    for repo in $REPOS; do
        local dir
        dir=$(repo_dir "$repo")
        if [ "$(current_branch "$repo")" != "$branch" ]; then
            if git -C "$dir" rev-parse -q --verify "refs/heads/$branch" >/dev/null; then
                git -C "$dir" checkout -q "$branch"
            else
                git -C "$dir" checkout -q -b "$branch" --track "origin/$branch"
            fi
        fi
        # Only fast forwards, so that commits made on this machine are never lost or merged by accident.
        git -C "$dir" merge -q --ff-only "origin/$branch" \
            || die "$repo: $branch has commits on this machine which are not on GitHub, push or remove them first"
        info "$repo: on $branch at $(git -C "$dir" log -1 --format='%h %s')"
    done
    info "Both repos are on $branch, build with: dx serve"
}

cmd_status() {
    local version state
    version=$(cargo_version "$(repo_dir marco-sparko)")
    echo "Version (marco-sparko Cargo.toml): $version"
    for repo in $REPOS; do
        local dir changes
        dir=$(repo_dir "$repo")
        git -C "$dir" fetch -q --tags origin
        changes=$(git -C "$dir" status --porcelain | wc -l | tr -d ' ')
        echo
        echo "$repo: $(current_branch "$repo"), version $(cargo_version "$dir"), $changes uncommitted changes"
        for tag in "test$version" "release$version"; do
            if remote_tag_exists "$repo" "$tag"; then
                echo "  $tag: $(git -C "$dir" rev-parse --short "$tag^{commit}" 2>/dev/null || echo "on GitHub only")"
            else
                echo "  $tag: none"
            fi
        done
    done
    require_gh
    state=$(release_state "release$version")
    echo
    echo "GitHub release release$version: $state"
    echo
    gh run list -R "$OWNER/marco-sparko" --workflow "$WORKFLOW" --limit 3
}

cmd_start() {
    local main_version new_version
    for repo in $REPOS; do
        require_clean "$repo"
        git -C "$(repo_dir "$repo")" fetch -q origin
    done
    main_version=$(git -C "$(repo_dir marco-sparko)" show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
    # The next minor version, e.g. 0.4.4 to 0.5.0, so that patch versions are left for fixes to the last release.
    new_version=${1:-$(echo "$main_version" | awk -F. '{ printf "%d.%d.0", $1, $2 + 1 }')}
    [[ "$new_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "$new_version is not a version like 0.4.4"
    info "Starting release $new_version (main is $main_version)"

    for repo in $REPOS; do
        local dir branch
        dir=$(repo_dir "$repo")
        branch="v$new_version"
        if git -C "$dir" rev-parse -q --verify "refs/heads/$branch" >/dev/null; then
            info "$repo: switching to the existing $branch"
            git -C "$dir" checkout -q "$branch"
        elif git -C "$dir" rev-parse -q --verify "refs/remotes/origin/$branch" >/dev/null; then
            info "$repo: checking out $branch from GitHub"
            git -C "$dir" checkout -q -b "$branch" --track "origin/$branch"
        else
            info "$repo: creating $branch from main"
            git -C "$dir" checkout -q -b "$branch" origin/main
        fi
    done

    info "Setting the version to $new_version"
    sed -i '' "1,/^version = /s/^version = .*/version = \"$new_version\"/" "$(repo_dir bruce-graphql)/Cargo.toml"
    sed -i '' -e "1,/^version = /s/^version = .*/version = \"$new_version\"/" \
        -e "s/^bruce-graphql = {version = \"[^\"]*\"/bruce-graphql = {version = \"$new_version\"/" \
        "$(repo_dir marco-sparko)/Cargo.toml"
    release_version >/dev/null

    # Updates the Cargo.lock files.
    for repo in $REPOS; do
        info "Building $repo"
        (cd "$(repo_dir "$repo")" && cargo build -q)
    done

    for repo in $REPOS; do
        local dir
        dir=$(repo_dir "$repo")
        if [ -n "$(git -C "$dir" status --porcelain)" ]; then
            git -C "$dir" commit -q -am "Start v$new_version"
        fi
        git -C "$dir" push -q -u origin "v$new_version"
        info "$repo: v$new_version pushed"
    done
    info "Release $new_version started, commit to the v$new_version branches"
}

cmd_try() {
    local version tag
    require_gh
    version=$(release_version)
    tag="test$version"
    for repo in $REPOS; do
        require_branch "$repo" "v$version"
        require_clean "$repo"
        require_pushed "$repo"
    done

    for repo in $REPOS; do
        info "$repo: tagging the release branch $tag"
        git -C "$(repo_dir "$repo")" tag -f "$tag"
        git -C "$(repo_dir "$repo")" push -q -f origin "$tag"
    done

    run_workflow "v$version" "$tag" false
    download_test_installers "$version" "$RUN_ID"
}

cmd_merge() {
    local version
    require_gh
    version=$(release_version)
    local tagged=0
    for repo in $REPOS; do
        require_branch "$repo" "v$version"
        require_clean "$repo"
        require_pushed "$repo"
        if remote_tag_exists "$repo" "release$version"; then
            tagged=$((tagged + 1))
        fi
    done
    [ "$tagged" -lt 2 ] || die "Both repos are already tagged release$version, run untag first to make more changes"

    for repo in $REPOS; do
        local dir
        dir=$(repo_dir "$repo")
        # Carries on after a merge which failed part way.
        if remote_tag_exists "$repo" "release$version"; then
            info "$repo: already merged and tagged release$version"
            continue
        fi
        if [ -z "$(gh pr list -R "$OWNER/$repo" --head "v$version" --state open --json number --jq '.[].number')" ]; then
            info "$repo: creating a pull request"
            gh pr create -R "$OWNER/$repo" --base main --head "v$version" --title "V$version" --body "Release $version"
        fi
        info "$repo: squash merging the pull request"
        gh pr merge "v$version" -R "$OWNER/$repo" --squash

        git -C "$dir" fetch -q origin
        info "$repo: tagging main release$version"
        git -C "$dir" tag "release$version" origin/main
        git -C "$dir" push -q origin "release$version"

        # A squash merge does not record that the branch's commits are on main, so merge it back so that the next pull
        # request only shows new changes. This changes no files.
        git -C "$dir" merge -q origin/main -m "Merge main after squash merge"
        git -C "$dir" push -q

        delete_tag "$repo" "test$version"
    done
    info "Merged and tagged release$version, next: $(basename "$0") check"
}

cmd_check() {
    local version
    version=$(release_version)
    for repo in $REPOS; do
        require_clean "$repo"
        remote_tag_exists "$repo" "release$version" || die "$repo has no release$version tag, run merge first"
        git -C "$(repo_dir "$repo")" fetch -q --tags origin
    done

    for repo in $REPOS; do
        info "$repo: checking out release$version"
        git -C "$(repo_dir "$repo")" checkout -q "release$version"
    done

    info "Clean build and run, quit the app when you have finished checking it"
    (cd "$(repo_dir marco-sparko)" && cargo clean && dx run --release) || echo "The app did not build or run"

    for repo in $REPOS; do
        local dir
        dir=$(repo_dir "$repo")
        # Cargo.lock may have been regenerated, e.g. by the editor, and is the committed one on the branch.
        git -C "$dir" restore Cargo.lock
        git -C "$dir" checkout -q "v$version"
        info "$repo: back on v$version"
    done
    info "If it was OK, next: $(basename "$0") build"
}

cmd_build() {
    local version tag state notes
    require_gh
    version=$(release_version)
    tag="release$version"
    for repo in $REPOS; do
        remote_tag_exists "$repo" "$tag" || die "$repo has no $tag tag, run merge first"
    done
    git -C "$(repo_dir marco-sparko)" fetch -q --tags origin

    state=$(release_state "$tag")
    case $state in
        published) die "$tag has already been published" ;;
        none)
            notes=$(changelog_notes_file "$version")
            info "Creating the draft release $tag, with the notes from the ChangeLog"
            gh release create "$tag" -R "$OWNER/marco-sparko" --draft --verify-tag --title "Release $version" \
                --notes-file "$notes"
            rm -f "$notes"
            ;;
        draft) info "The draft release $tag already exists, its installers will be replaced (and not its notes, see notes)" ;;
    esac

    # The workflow from the tagged commit, so it is the one which was released.
    run_workflow "$tag" "$tag" true
    download_release_installers "$version"
}

cmd_watch() {
    local version id ref
    require_gh
    version=$(release_version)
    id=$(latest_run_id)
    [ -n "$id" ] || die "There are no runs of the $WORKFLOW workflow"
    # The branch or tag the run was started from, which shows whether it was started by try or build.
    ref=$(gh run view "$id" -R "$OWNER/marco-sparko" --json headBranch --jq .headBranch)
    case $ref in
        "v$version")
            watch_run "$id"
            download_test_installers "$version" "$id"
            ;;
        "release$version")
            watch_run "$id"
            download_release_installers "$version"
            ;;
        *) die "The latest run ($id) is from $ref, not from try or build of $version" ;;
    esac
}

# Downloads the installer for this machine from the latest run, from the run's artifact for try, or from the draft (or
# published) release for build, to the same place as try and build download all of them.
cmd_download() {
    local version id ref status platform download
    require_gh
    version=$(release_version)
    platform=$(this_platform)
    id=$(latest_run_id)
    [ -n "$id" ] || die "There are no runs of the $WORKFLOW workflow"
    ref=$(gh run view "$id" -R "$OWNER/marco-sparko" --json headBranch --jq .headBranch)
    status=$(gh run view "$id" -R "$OWNER/marco-sparko" --json status --jq .status)
    [ "$status" = "completed" ] || die "The latest run ($id) is $status, wait for it with: $(basename "$0") watch"
    case $ref in
        "v$version")
            download="$HOME/Downloads/marco-sparko-$version-test/$platform"
            rm -rf "$download"
            info "Downloading $platform from run $id"
            gh run download "$id" -R "$OWNER/marco-sparko" -n "$platform" --dir "$download"
            ;;
        "release$version")
            download="$HOME/Downloads/marco-sparko-$version"
            info "Downloading $(installer_pattern "$platform") from release$version"
            gh release download "release$version" -R "$OWNER/marco-sparko" -p "$(installer_pattern "$platform")" \
                --dir "$download" --clobber
            ;;
        *) die "The latest run ($id) is from $ref, not from try or build of $version" ;;
    esac
    info "The installer is in $download"
}

cmd_notes() {
    local version tag notes
    require_gh
    version=$(release_version)
    tag="release$version"
    [ "$(release_state "$tag")" = "draft" ] || die "There is no draft release $tag, run build first"
    git -C "$(repo_dir marco-sparko)" fetch -q --tags origin
    notes=$(changelog_notes_file "$version")
    gh release edit "$tag" -R "$OWNER/marco-sparko" --notes-file "$notes"
    rm -f "$notes"
    info "The notes of $tag are now the ChangeLog's:"
    gh release view "$tag" -R "$OWNER/marco-sparko" --json url --jq .url
}

cmd_publish() {
    local version tag assets missing=""
    require_gh
    version=$(release_version)
    tag="release$version"
    [ "$(release_state "$tag")" = "draft" ] || die "There is no draft release $tag, run build first"

    assets=$(gh release view "$tag" -R "$OWNER/marco-sparko" --json assets --jq '.assets[].name')
    for installer in $INSTALLERS; do
        installer=${installer//VERSION/$version}
        echo "$assets" | grep -qx "$installer" || missing="$missing $installer"
    done
    [ -z "$missing" ] || die "The draft release is missing:$missing"

    gh release edit "$tag" -R "$OWNER/marco-sparko" --draft=false --latest
    info "Published:"
    gh release view "$tag" -R "$OWNER/marco-sparko" --json url --jq .url
    info "Next: $(basename "$0") start"
}

cmd_untag() {
    local version tag state
    require_gh
    version=$(release_version)
    tag="release$version"
    state=$(release_state "$tag")
    [ "$state" != "published" ] || die "$tag has been published, so start a new version instead"
    if [ "$state" = "draft" ]; then
        info "Deleting the draft release $tag"
        gh release delete "$tag" -R "$OWNER/marco-sparko" --yes
    fi
    for repo in $REPOS; do
        info "$repo: deleting $tag"
        delete_tag "$repo" "$tag"
    done
    info "Make the changes on the v$version branches, then run merge again"
}

# In braces, with exit, so that bash reads all of this before running it, and never reads any more of the file, which
# sync may change, e.g. when it pulls a new version of this script.
{
[ $# -ge 1 ] || usage
command=$1
shift
case $command in
    status) cmd_status ;;
    start) cmd_start "$@" ;;
    try) cmd_try ;;
    merge) cmd_merge ;;
    check) cmd_check ;;
    build) cmd_build ;;
    notes) cmd_notes ;;
    publish) cmd_publish ;;
    untag) cmd_untag ;;
    watch) cmd_watch ;;
    download) cmd_download ;;
    sync) cmd_sync "$@" ;;
    *) usage ;;
esac
exit
}
