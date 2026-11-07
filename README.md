# microbit

Rust firmware for the [BBC micro:bit](https://microbit.org). The v2 is the board I'm writing for, and everything builds for the v1 too. Each firmware is its own folder under `firmware/`, on top of a small `board` crate that hides which micro:bit it's running on.

> [!NOTE]
> Nothing here has run on a real micro:bit yet. It builds, and the logic is tested on the host, and that's all.

## The boards

| | v1 (1.3, 1.5) | v2 (2.0, 2.2x) |
|---|---|---|
| Chip | nRF51822, Cortex-M0, 16 MHz | nRF52833, Cortex-M4F, 64 MHz |
| Flash, RAM | 256 KB, 16 KB | 512 KB, 128 KB |
| Rust target | `thumbv6m-none-eabi` | `thumbv7em-none-eabihf` |
| probe-rs chip | `nRF51822_xxAA` | `nRF52833_xxAA` |
| Extras | | speaker, microphone, touch logo, power LED |

The v2 has a notched edge connector and a speaker on the back, which is the quickest way to tell them apart. [docs/hardware.md](docs/hardware.md) has the rest, with a source for each fact.

## Firmware

| Name | What it does |
|---|---|
| [heartbeat](firmware/heartbeat) | A heart beating on the 5×5 matrix. A slows it down, B speeds it up. |

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

Every micro:bit has a debug probe on its USB port (the "interface chip", running Arm's DAPLink), so all you need is a USB cable and [probe-rs](https://probe.rs).

```sh
cargo install probe-rs-tools --locked
cargo xtask flash heartbeat             # v2
cargo xtask flash heartbeat --board v1  # v1
```

That runs `cargo flash` with the right chip, target and feature for the board, so the only thing to get right is `--board`.

No probe-rs? Plug the micro:bit in, and a drive called MICROBIT turns up. Drag `target/firmware/heartbeat-v2.hex` (or `-v1`) onto it. The yellow LED by the USB port flickers while it writes, then the drive ejects itself and the firmware starts. Each hex is for one board only, so pick the right one: the micro:bit Foundation says a v1 hex on a v2 gives an error, and a v2 hex on a v1 fails without saying anything.

[docs/flashing.md](docs/flashing.md) covers installing probe-rs on each OS, updating the interface firmware and what to do when the probe or the drive won't show up.

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) has the checks, the layout and how to add a firmware.

## Licence

MIT.
