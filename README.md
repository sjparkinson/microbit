# microbit

Rust firmware for the [BBC micro:bit](https://microbit.org). The v2 is the board I'm writing for, and everything builds for the v1 too. Each firmware is its own folder under `firmware/`, on top of a small `board` crate that hides which micro:bit it's running on.

## The boards

| | v1 (1.3, 1.5) | v2 (2.0, 2.2x) |
|---|---|---|
| Chip | nRF51822, Cortex-M0, 16 MHz | nRF52833, Cortex-M4F, 64 MHz |
| Flash, RAM | 256 KB, 16 KB | 512 KB, 128 KB |
| Rust target | `thumbv6m-none-eabi` | `thumbv7em-none-eabihf` |
| Extras | | speaker, microphone, touch logo, power LED |

The v2 has a notched edge connector and a speaker on the back, which is the quickest way to tell them apart. [docs/hardware.md](docs/hardware.md) has the rest, with a source for each fact.

## Firmware

| Name | What it does |
|---|---|
| [heartbeat](firmware/heartbeat) | A heart beating on the 5×5 matrix. A slows it down, B speeds it up. |
| [hello](firmware/hello) | HELLO WORLD! scrolling across the matrix, over and over. |
| [life](firmware/life) | Conway's Game of Life on a wrapping 5×5 grid, with a new random soup once it settles. A throws a new soup, B pauses. |

`cargo xtask list` prints the same list from what's in `firmware/`.

## Building

You need [rustup](https://rustup.rs). The pinned stable toolchain and both thumb targets come down on the first build.

```sh
cargo xtask build heartbeat             # for the v2
cargo xtask build heartbeat --board v1  # for the v1
cargo xtask build --all --all-boards    # everything, for both
cargo test --workspace
```

`build` prints how much flash and RAM each image uses, and writes the ELF and a `.hex` to `target/firmware/<name>-<board>.hex`.

## Flashing

Nothing to install. Plug the micro:bit in with a data cable.

```sh
cargo xtask flash heartbeat             # v2
cargo xtask flash heartbeat --board v1  # v1
```

The interface chip on every micro:bit's USB port is a CMSIS-DAP debug probe, and `xtask` speaks that itself, with no probe-rs or OpenOCD. It halts the chip, erases the pages the firmware needs, writes it, reads every word back, and resets it. It checks the board ID in the USB serial number first, and the chip's CPU type once it's connected, so a v2 image never goes on a v1.

[docs/flashing.md](docs/flashing.md) has how it works, updating the interface firmware and what to do when it won't connect.

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) has the checks, the layout and how to add a firmware.

## Licence

MIT.
