#!/bin/sh
# Builds the virtual capture device: the capture firmware for the board simulator, once for each board it simulates,
# and its launcher, into target-sim/bin, for this computer, e.g.
#   ./build-sim.sh            then   target-sim/bin/bruce-smart-home-capture-sim --board rlcd42
# with the stable toolchain and this computer's own target, as the firmware's settings are for the ESP32s. See
# bruce-embedded-std's docs/Simulator.md. Give --debug for a debug build.
set -e
cd "$(dirname "$0")"
profile=release
flag=--release
if [ "$1" = "--debug" ]; then
    profile=debug
    flag=
fi
host=$(rustc +stable -vV | sed -n 's/^host: //p')
case "$host" in
    *windows*) exe=.exe ;;
    *) exe= ;;
esac
mkdir -p target-sim/bin
# The developer's own links to libraries' checkouts, if any, see .cargo/links.toml.
links=
if [ -f .cargo/links.toml ]; then
    links="--config .cargo/links.toml"
fi
# Copies a program as a new file, removing any before it, as macOS kills a program copied over another, which keeps the
# old file's identity, with the new file's code signature.
copy() {
    rm -f "$2"
    cp "$1" "$2"
}
for pair in wave-esp32s3-rlcd-42:rlcd42 guition-jc3248w535c:guitron; do
    feature=${pair%%:*}
    name=${pair##*:}
    echo "Building the virtual $name"
    CARGO_TARGET_DIR="target-sim/$name" cargo +stable build $links $flag --no-default-features --features "sim,$feature" \
        --target "$host"
    copy "target-sim/$name/$host/$profile/bruce-smart-home-capture$exe" "target-sim/bin/bruce-smart-home-capture-$name$exe"
done
copy "target-sim/rlcd42/$host/$profile/bruce-smart-home-capture-sim$exe" "target-sim/bin/bruce-smart-home-capture-sim$exe"
# Copies named as dx bundle's external_bin needs them, with this computer's target, for the app's installers, which
# put them beside the app's own program, without it, see Dioxus.toml.
mkdir -p target-sim/bundle
for program in sim rlcd42 guitron; do
    copy "target-sim/bin/bruce-smart-home-capture-$program$exe" "target-sim/bundle/bruce-smart-home-capture-$program-$host$exe"
    # On macOS, signed ad hoc, as an Intel Mac's linker, unlike an Apple Silicon one's, leaves a program unsigned, and
    # codesign will not sign the app's own program beside an unsigned one; dx bundle then signs the whole app, these
    # with it, with the Developer ID, if it has one.
    case "$host" in
        *apple-darwin) codesign --force --sign - "target-sim/bundle/bruce-smart-home-capture-$program-$host" ;;
    esac
done
echo "Built target-sim/bin and target-sim/bundle"
