# CLAUDE.md

Rust firmware for the BBC micro:bit, v2 (nRF52833) first and v1 (nRF51822) too. Each firmware is a crate in `firmware/`, on a `board` crate that wraps the nrf-rs BSPs. Read README.md for what it is, CONTRIBUTING.md for the checks and layout, and docs/ for the hardware and flashing.

## Commands

```sh
cargo fmt --check                                               # formatting, as CI runs it
cargo test --workspace --locked -q                              # host tests, run these constantly
cargo clippy --workspace --all-targets --locked -- -D warnings  # host lints
cargo xtask check                                               # clippy and builds for both boards, what CI runs last
cargo xtask build <name> [--board v1|v2]                        # ELF, hex, and flash/RAM use
cargo xtask flash <name> [--board v1|v2]                        # copy the hex onto the MICROBIT drive
```

Plain `cargo test` only tests the root package, which has nothing in it, so always pass `--workspace`. `--board` defaults to v2.

## Agents

- `hardware-researcher`: before writing a new fact into docs/hardware.md, or when code needs a pin or number nobody's sourced. It reports and never edits.

## Rules that bite

- **Logic goes in the firmware's `src/lib.rs`**, which is `no_std` and tested on the host. `src/main.rs` stays a thin loop between `board` and the library.
- **Every firmware builds for both boards.** Anything v2-only (speaker, microphone, touch logo) is gated behind the `v2` feature and its doc comment says what the v1 gets. `cargo xtask check` is what proves a change builds on both; host tests don't.
- **Hardware differences live in `board`**, not in `#[cfg(feature = ...)]` scattered through firmware.
- **The v1 has 16 KB of RAM and no FPU.** Check `cargo xtask build <name> --board v1` after anything that adds buffers or floats.
- **Edit `.rs` files with Edit or Write**, not `sed -i`, `cp` or a heredoc. The lint hook only runs on Edit and Write.
- **Flashing needs a micro:bit on USB.** Flashing can't damage the board, so running `cargo xtask flash` is fine when the user's asked for it. It copies the hex onto the MICROBIT drive, outside the repo, so the sandbox blocks it. Give the user the command rather than retrying. There's no probe-rs or `cargo flash` here, on purpose; don't add them back.
- **Nothing has run on hardware until the user says it has.** A build that passes isn't a board that works. Don't write it up as if it were.
- **Mark uncertainty.** docs/hardware.md labels facts confirmed or likely. Keep doing that, and don't promote a guess.
- **Git writes need the sandbox off** in this repo, since it denies writes to `.git`. Commits are single-line Conventional Commits, straight to main, and the pre-commit hook runs the host tests first.

## Things that look wrong but aren't

- Each firmware's `src/main.rs` has a host `fn main()` that prints a message and exits with an error. It's there so the workspace builds and tests on the host, where `board` is only its types.
- Every firmware has its own `build.rs` that adds `-Tlink.x`. A build script's link arguments only reach its own package, so `board` can't do it for them.
- `.cargo/config.toml` has no `[build] target`. The host is the default on purpose, so `cargo test --workspace` works; xtask passes the thumb target.
