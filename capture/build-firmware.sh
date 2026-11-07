#!/bin/sh
# Builds the capture firmware for every board, or for those named, e.g. ./build-firmware.sh wave-esp32s3-rlcd-42, and
# copies each to the shared firmware folder as soon as it is built, as bruce-smart-home's dev version, see
# bruce-embedded-firmware in sparko-embedded-std, where the app's Devices page and bruce-embedded-flash find it. Boards
# with the same chip build to the same path in target/, so the last built would otherwise be the only one. Each file's
# firmware record says which board it is for, so a file cannot be flashed to another board unnoticed.
#
# ./build-firmware.sh --path <board> prints where a board's firmware is put, e.g. for bruce-embedded-flash flash.
set -e
cd "$(dirname "$0")"

# The shared firmware folder, as bruce-embedded-firmware's folder() finds it.
if [ -n "$BRUCE_EMBEDDED_FIRMWARE" ]; then
    folder=$BRUCE_EMBEDDED_FIRMWARE
elif [ "$(uname)" = Darwin ]; then
    folder="$HOME/Library/Application Support/bruce-embedded-std/firmware"
else
    folder="${XDG_DATA_HOME:-$HOME/.local/share}/bruce-embedded-std/firmware"
fi
dev="$folder/bruce-smart-home/dev"

if [ "$1" = --path ]; then
    echo "$dev/marco-sparko-capture-$2.elf"
    exit
fi
mkdir -p "$dev"

# Each board, its MCU and its target.
ALL="cyd-esp32-28:esp32:xtensa-esp32-espidf
wave-esp32c6-touch-147:esp32c6:riscv32imac-esp-espidf
wave-esp32s3-rlcd-42:esp32s3:xtensa-esp32s3-espidf
guition-jc3248w535c:esp32s3:xtensa-esp32s3-espidf"

for entry in $ALL; do
    board=${entry%%:*}
    rest=${entry#*:}
    mcu=${rest%%:*}
    target=${rest#*:}
    if [ $# -gt 0 ] && ! echo " $* " | grep -q " $board "; then
        continue
    fi
    echo "Building the capture firmware for $board"
    MCU=$mcu cargo build --release --target "$target" --no-default-features --features "$board"
    cp "target/$target/release/marco-sparko-capture" "$dev/marco-sparko-capture-$board.elf"
done
ls -l "$dev"
