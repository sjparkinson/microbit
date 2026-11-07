# Flashing

There's nothing to install and no programmer to buy. The micro:bit's USB port goes to an interface chip running DAPLink, which shows up on your computer as a drive called MICROBIT. Copy a `.hex` onto it and the interface chip programs the target ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). A data-capable USB cable is all you need. Some cheap micro-USB cables only carry power, and with one of those nothing shows up at all.

Flashing can't hurt the board. The interface chip is separate from the target, so "no matter what code you run on your micro:bit, or how you manage to crash the device, you can always still put a new program on using the USB connection" (same page).

## cargo xtask flash

```sh
cargo xtask flash heartbeat             # v2
cargo xtask flash heartbeat --board v1  # v1
```

That finds the MICROBIT drive, checks it's the board you asked for, builds a release image, and copies `target/firmware/heartbeat-v2.hex` (or `-v1`) onto it. It looks for the drive at `/Volumes/MICROBIT` on macOS, and `/media/$USER/MICROBIT` or `/run/media/$USER/MICROBIT` on Linux. Anywhere else, pass `--drive <path>`.

The board check reads the "Unique ID" line in the drive's `DETAILS.TXT`, whose first four digits are a board ID. The micro:bit Foundation's [universal hex code](https://github.com/microbit-foundation/microbit-universal-hex/blob/main/src/universal-hex.ts) gives 9900 and 9901 for the v1 and 9903 for the v2. If the drive says one board and you asked for the other, `flash` stops before copying anything. If it can't make sense of the file it warns and carries on. That check matters more than it looks, because of the next bit.

## By hand

```sh
cargo xtask build heartbeat             # writes target/firmware/heartbeat-v2.hex
cargo xtask build heartbeat --board v1  # writes target/firmware/heartbeat-v1.hex
```

Plug the micro:bit in and drag the hex onto the MICROBIT drive. The yellow LED next to the USB port flashes while it writes, then the drive ejects itself and comes back, and the firmware runs. The drive's not real storage: a file dropped on it is streamed into the target and doesn't stay there ([DAPLink page](https://tech.microbit.org/software/daplink-interface/#files-on-the-microbit-drive)). If your computer complains the disk wasn't ejected properly, that's the drive ejecting itself, and it's fine.

Each hex is for one board. The micro:bit Foundation's universal hex holds both, but `xtask` doesn't make those. Their [hex format page](https://tech.microbit.org/software/hex-format/#cross-device-compatibility) says "If a V1 only .hex is detected on a V2 board it will throw an error, but a V2 only hex will fail silently on a V1." So if a v1 does nothing after a drag, check you dragged the `-v1` file.

## Updating the interface firmware

Worth doing once on any board you didn't buy new. The current versions are 0253 for every v1, 0255 for a 2.00 and 0257 for a 2.2x ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). `DETAILS.TXT` on the MICROBIT drive says which one you've got.

1. Download the right file for your board from [microbit.org's firmware page](https://microbit.org/get-started/user-guide/firmware/). The page tells you how to identify the revision.
2. Unplug the micro:bit. Hold the reset button on the back, and plug it in while still holding it. A drive called MAINTENANCE appears instead of MICROBIT.
3. Drag the firmware hex onto MAINTENANCE and wait for the yellow LED to stop flashing. The board resets and comes back as MICROBIT.

Only use firmware from microbit.org. The DAPLink page warns that firmware from anywhere else "could damage your micro:bit, or make it impossible to re-flash", and that's the one way to break the USB side.

## When it doesn't work

- **The drive shows up as MAINTENANCE.** The reset button was held while it powered up. Unplug and plug back in without touching it. `cargo xtask flash` spots this and says so.
- **Neither drive appears.** Probably a power-only cable, or a dead USB port. The red power LED on a v2 only says the board's powered, not that the data lines work.
- **A `FAIL.TXT` appears on the drive.** DAPLink couldn't flash the hex, and the file says why. The Foundation keeps a [list of the error codes](https://support.microbit.org/en/support/solutions/articles/19000016969-micro-bit-error-codes) and DAPLink's [`error.c`](https://github.com/mbedmicro/DAPLink/blob/master/source/daplink/error.c) has the source of each message. A v1 hex on a v2 is one way to get one.

## Sandbox note

Claude Code's sandbox can't write outside the repo, so `cargo xtask flash` fails there when it copies to the drive. If it does, run the command yourself.
