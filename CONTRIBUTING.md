# Contributing

## Checks

CI runs these on every push, so run them first.

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo xtask check
```

The first three run on your own machine's target, where every firmware's `main.rs` is a stub and only the libraries do anything. `cargo xtask check` is the one that builds for the micro:bits: clippy on each thumb target, then every firmware for both boards. A firmware that only compiles on the host gets caught there.

Plain `cargo test` only tests the root package, which here is nothing, so pass `--workspace`.

## Layout

```
board/                  one API over both boards, on top of the nrf-rs BSPs
  src/lib.rs            what every build sees, host included (the Image type)
  src/hw.rs             the board itself: display, buttons, timer (thumb targets only)
firmware/<name>/        one crate per firmware
  src/lib.rs            no_std logic, tested on the host
  src/main.rs           a thin loop between board and the library (a stub on the host)
  build.rs              links with cortex-m-rt's link.x
xtask/                  list, build, flash, check; no dependencies
docs/                   hardware, flashing
```

`board` has features `v1` and `v2`. With `v1` it pulls in the [`microbit`](https://crates.io/crates/microbit) crate, with `v2` [`microbit-v2`](https://crates.io/crates/microbit-v2), and with neither it's only the types the host tests need. Each firmware has the same two features, passed straight through.

Keep logic in the firmware's `src/lib.rs`, where `cargo test` reaches it. `main.rs` should read the buttons, call the library and show what comes back, and not much else.

## Adding a firmware

1. Copy `firmware/heartbeat` to `firmware/<name>`.
2. Change `name` in its `Cargo.toml`, in both `[package]` and `[[bin]]`, and the `description`.
3. Replace `src/lib.rs` and `src/main.rs` with your own. Keep `build.rs` as it is: a build script's link arguments only reach its own package, so every firmware needs one.
4. Add a row to the firmware table in README.md.

The workspace takes every folder in `firmware/*`, and `cargo xtask` finds firmware by listing `firmware/*/Cargo.toml`, so nothing else needs telling. `cargo xtask list` should show it straight away.

## v1 and v2

Every firmware builds for both boards. If something only exists on the v2 (the speaker, the microphone, the touch logo), either do without it on the v1 or gate it behind the `v2` feature in `board`, and say in a doc comment what the v1 gets instead.

A firmware that can't work on a v1 at all (a sound meter, say) has to say so at the top of its `src/lib.rs` and in the README's table, and `xtask` has to be told so `check` stops building it for the v1. There's no way to tell it yet. Add one when the first v2-only firmware arrives, rather than guessing at its shape now.

The v1 is the tighter of the two: a Cortex-M0 with 16 KB of RAM and no FPU, so floats are slow software routines there. Watch the RAM figure `cargo xtask build --board v1` prints.

[docs/hardware.md](docs/hardware.md) has what's different between the boards, and how sure anyone is about each bit.

## Unsafe

`unsafe_code` is denied across the workspace. The BSPs and HALs do the register work, so there shouldn't be a reason to turn it back on. If there is, every `unsafe` block gets a `// SAFETY:` comment.

## Commits

Single-line [Conventional Commits](https://www.conventionalcommits.org), such as `feat(heartbeat): flash the heart on a press`. Use the firmware's folder name as the scope, or `board` or `xtask`. They go straight to main, so each one should pass the checks above on its own.
