//! Links the firmware with cortex-m-rt's `link.x`, which pulls in the
//! `memory.x` the HAL writes out (nrf51-hal for the v1, nrf52833-hal for the
//! v2). Every firmware crate needs this file: a build script's link arguments
//! only reach its own package's binaries, so `board` can't do it for them.

use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    // The host build is only there for tests, so it links normally.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    println!("cargo::rustc-link-arg-bins=-Tlink.x");
}
