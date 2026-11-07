#!/usr/bin/env bash
# Updates a micro:bit's interface firmware on macOS, where dragging the .hex
# onto MAINTENANCE fails with "The transfer timed out". It writes the .hex
# straight onto the drive's data area, past the macOS FAT driver. DAPLink
# spots a hex by its content as the sectors arrive, so no directory entry is
# needed. docs/flashing.md has the story.
#
# Usage: hold reset while plugging the micro:bit in, so MAINTENANCE appears, then
#   sudo bash scripts/daplink-update.sh ~/Downloads/0249_kl26z_microbit_0x8000.hex
set -euo pipefail

hex=${1:?usage: sudo bash $0 <firmware.hex>}
vol=/Volumes/MAINTENANCE

[[ $(uname) == Darwin ]] || { echo "this is for macOS; elsewhere, drag the hex onto MAINTENANCE" >&2; exit 1; }
[[ $EUID -eq 0 ]] || { echo "run it with sudo" >&2; exit 1; }
[[ -f $hex ]] || { echo "no such file: $hex" >&2; exit 1; }
[[ -d $vol ]] || { echo "$vol isn't mounted; hold reset while plugging the micro:bit in" >&2; exit 1; }
grep -q "DAPLink" "$vol/DETAILS.TXT" || { echo "$vol doesn't look like a micro:bit's DAPLink drive" >&2; exit 1; }

info=$(diskutil info "$vol")
node=$(awk -F': *' '/Device Node:/ {print $2}' <<<"$info")
bytes=$(sed -nE 's/.*Disk Size:.*\(([0-9]+) Bytes\).*/\1/p' <<<"$info")
grep -q "Protocol: *USB" <<<"$info" || { echo "$node isn't a USB disk; stopping" >&2; exit 1; }
grep -q "Removable Media: *Removable" <<<"$info" || { echo "$node isn't removable; stopping" >&2; exit 1; }
(( bytes < 200000000 )) || { echo "$node is $bytes bytes, too big to be DAPLink; stopping" >&2; exit 1; }
raw=${node/\/dev\/disk//dev/rdisk}

# FAT16 boot sector fields, little-endian. The raw device only takes whole
# sectors, so copy the first one out and read that. macOS od ends with an
# empty line, hence the NF and exit.
boot=$(mktemp)
trap 'rm -f "$boot"' EXIT
dd if="$raw" of="$boot" bs=512 count=1 2>/dev/null
u16() { od -An -tu1 -j "$1" -N2 "$boot" | awk 'NF {print $1 + 256 * $2; exit}'; }
u8() { od -An -tu1 -j "$1" -N1 "$boot" | awk 'NF {print $1; exit}'; }
bps=$(u16 11)
reserved=$(u16 14)
fats=$(u8 16)
root_entries=$(u16 17)
per_fat=$(u16 22)
(( bps == 512 )) || { echo "sector size $bps, expected 512; stopping" >&2; exit 1; }
data=$(( reserved + fats * per_fat + (root_entries * 32 + bps - 1) / bps ))

echo "device $node ($bytes bytes), data area starts at sector $data"
read -r -p "write $(basename "$hex") there? type yes: " answer
[[ $answer == yes ]] || { echo "stopped"; exit 1; }

diskutil unmountDisk "$node"
dd if="$hex" of="$raw" bs=512 seek="$data" conv=sync
sync
echo "written; replug the micro:bit without touching reset and MICROBIT should come back"
