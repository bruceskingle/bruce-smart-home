#!/bin/sh
# Links as cc does, then signs the executable linked with the identity named by DEBUG_SIGNING_IDENTITY, if it is set,
# so that every debug build of the app carries the same signature. The Keychain allows a program to read its items by
# its signature, and an ad hoc one, as cargo's builds have, is new with each build, so macOS asked to allow the app to
# read its secrets' key the first time each build ran. Used as the linker by a git-ignored .cargo/config.toml, which
# names the identity, as signing settings are kept out of the repository, see docs/developers.md.
set -e
cc "$@"
[ -n "$DEBUG_SIGNING_IDENTITY" ] || exit 0
output=""
while [ $# -gt 0 ]; do
    if [ "$1" = "-o" ]; then
        output="$2"
        shift
    fi
    shift
done
# Only executables, not libraries, e.g. proc macros, which the Keychain never sees.
case "$output" in
    *.dylib | *.so | *.rlib | "") exit 0 ;;
esac
# The identifier is the file's name without the hash cargo adds, e.g. bruce_smart_home, not bruce_smart_home-6e46...,
# as the hash changes with the dependencies, and the Keychain's permission names the identifier as well as the identity.
identifier=$(basename "$output" | sed -E 's/-[0-9a-f]{16}$//')
codesign --force --sign "$DEBUG_SIGNING_IDENTITY" --identifier "$identifier" "$output" 2>/dev/null \
    || echo "link-and-sign.sh: unable to sign $output" >&2
