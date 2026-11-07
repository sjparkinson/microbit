# Flashing

There's no programmer to buy. The micro:bit's USB port goes to an interface chip running DAPLink, which is a CMSIS-DAP debug probe wired to the target's SWD pins, as well as the MICROBIT drive ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). A data-capable USB cable is all you need. Some cheap micro-USB cables only carry power, and with one of those nothing shows up at all.

Flashing can't hurt the board. The interface chip is separate from the target, so "no matter what code you run on your micro:bit, or how you manage to crash the device, you can always still put a new program on using the USB connection" (same page).

## Installing probe-rs

[probe.rs](https://probe.rs/docs/getting-started/installation/) lists a few ways. Any of them gives you `probe-rs`, `cargo flash` and `cargo embed`.

```sh
# the installer script (macOS, Linux)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/probe-rs/probe-rs/releases/latest/download/probe-rs-tools-installer.sh | sh

# Homebrew
brew tap probe-rs/probe-rs
brew install probe-rs

# from source
cargo install probe-rs-tools --locked
```

macOS and Windows need nothing else. Building from source on Linux needs `pkg-config libudev-dev cmake git` (Debian and Ubuntu) or `libusbx-devel libudev-devel cmake git` (Fedora) first.

On Linux you also need udev rules, or the probe is only usable as root ([probe setup](https://probe.rs/docs/getting-started/probe-setup/#linux-udev-rules)):

```sh
curl -LO https://probe.rs/files/69-probe-rs.rules
sudo mv 69-probe-rs.rules /etc/udev/rules.d/
sudo udevadm control --reload
sudo udevadm trigger
```

The rules give access to the `plugdev` group, so if it still doesn't work, check you're in it (`sudo groupadd --system plugdev && sudo usermod -a -G plugdev $USER`, then log out and back in). The rules file has an entry for DAPLink's USB ID, `0d28:0204`.

## Flashing with cargo flash

```sh
cargo xtask flash heartbeat             # v2
cargo xtask flash heartbeat --board v1  # v1
```

That builds a release image and runs

```sh
cargo flash --release --chip nRF52833_xxAA --target thumbv7em-none-eabihf -p heartbeat --features v2
cargo flash --release --chip nRF51822_xxAA --target thumbv6m-none-eabi -p heartbeat --features v1
```

for the v2 and v1. You can run those yourself too. `cargo flash` erases what it needs, writes, and resets the chip, so the firmware starts straight away.

`cargo run --release -p heartbeat --target thumbv7em-none-eabihf --features v2` also works. `.cargo/config.toml` sets `probe-rs run` as the runner for both targets, which flashes and then stays attached for RTT output.

## Without probe-rs: the MICROBIT drive

```sh
cargo xtask build heartbeat             # writes target/firmware/heartbeat-v2.hex
cargo xtask build heartbeat --board v1  # writes target/firmware/heartbeat-v1.hex
```

Plug the micro:bit in and drag the hex onto the MICROBIT drive. The yellow LED next to the USB port flashes while it writes, then the drive ejects itself and comes back, and the firmware runs. The drive's not real storage: a file dropped on it is streamed into the target and doesn't stay there ([DAPLink page](https://tech.microbit.org/software/daplink-interface/#files-on-the-microbit-drive)). If your computer complains the disk wasn't ejected properly, that's the drive ejecting itself, and it's fine.

Each hex is for one board. The micro:bit Foundation's universal hex holds both, but `xtask` doesn't make those. Their [hex format page](https://tech.microbit.org/software/hex-format/#cross-device-compatibility) says "If a V1 only .hex is detected on a V2 board it will throw an error, but a V2 only hex will fail silently on a V1." So if a v1 does nothing after a drag, check you dragged the `-v1` file.

## Updating the interface firmware

Worth doing once on any board you didn't buy new, and on any v1 before blaming probe-rs. The current versions are 0253 for every v1, 0255 for a 2.00 and 0257 for a 2.2x ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). `DETAILS.TXT` on the MICROBIT drive says which one you've got.

1. Download the right file for your board from [microbit.org's firmware page](https://microbit.org/get-started/user-guide/firmware/). The page tells you how to identify the revision.
2. Unplug the micro:bit. Hold the reset button on the back, and plug it in while still holding it. A drive called MAINTENANCE appears instead of MICROBIT.
3. Drag the firmware hex onto MAINTENANCE and wait for the yellow LED to stop flashing. The board resets and comes back as MICROBIT.

Only use firmware from microbit.org. The DAPLink page warns that firmware from anywhere else "could damage your micro:bit, or make it impossible to re-flash", and that's the one way to break the USB side.

## When it doesn't work

- **probe-rs can't find a probe.** Try `probe-rs list`. If it's empty, check the cable carries data (does the MICROBIT drive appear?), then on Linux the udev rules above. Under WSL the probe has to be passed through with usbipd, and that's been reported not to work ([probe-rs #2351](https://github.com/probe-rs/probe-rs/issues/2351)).
- **The drive shows up as MAINTENANCE.** The reset button was held while it powered up. Unplug and plug back in without touching it.
- **Neither drive appears.** Probably a power-only cable, or a dead USB port. The red power LED on a v2 only says the board's powered, not that the data lines work.
- **A `FAIL.TXT` appears on the drive after a drag.** DAPLink couldn't flash the hex, and the file says why. The Foundation keeps a [list of the error codes](https://support.microbit.org/en/support/solutions/articles/19000016969-micro-bit-error-codes) and DAPLink's [`error.c`](https://github.com/mbedmicro/DAPLink/blob/master/source/daplink/error.c) has the source of each message. A v1 hex on a v2 is one way to get one.
- **Flashing works, then RTT drops out after a while on a 2.20.** That's [probe-rs #1235](https://github.com/probe-rs/probe-rs/issues/1235), still open. It doesn't affect `cargo flash`.
- **probe-rs on a v1.** I couldn't find any reported problems with the KL26's DAPLink and probe-rs, or anyone saying it works. nrf-rs's README shows its examples running on both with probe-rs, so I'd expect it to, after a firmware update to 0253. Nothing here has tried yet.

## Sandbox note

Claude Code's sandbox may block access to USB devices, so `cargo xtask flash` can fail there with a "no probe found" that isn't true. If it does, run the command yourself.
