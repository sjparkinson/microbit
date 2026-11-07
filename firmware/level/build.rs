//! Links the firmware with cortex-m-rt's `link.x`, which pulls in the
//! `memory.x` the HAL writes out. A copy of hello's: a build script's
//! link arguments only reach its own package's binaries.

use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    // The host build is only there for tests, so it links normally.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    println!("cargo::rustc-link-arg-bins=-Tlink.x");
}
