//! The firmware's half of keeping the debug port open on an nRF52833 with
//! the hardened access port protection (revision 2, build code Bxx and
//! later). Those lock the port at every pin or power reset unless
//! UICR.APPROTECT is HwDisabled, which `cargo xtask flash` writes, *and* the
//! firmware writes APPROTECT.DISABLE as it boots, which is this.
//!
//! It does what Nordic's own startup code does (nrfx, `system_nrf52_approtect.h`,
//! `nrf52_handle_approtect`): on a hardened chip only, copy UICR.APPROTECT
//! into APPROTECT.DISABLE, so the UICR stays in charge. docs/hardware.md has
//! the sources.
//!
//! Raw register access, because the nRF52833 PAC has no APPROTECT
//! peripheral and the revision words aren't in its FICR. That makes this
//! the one file in `board` with `unsafe`.

#![allow(unsafe_code)]

use core::ptr;

/// Undocumented FICR words nrfx's `nrf52_configuration_249` reads: 0x0D for
/// the nRF52833, then 0 for revision 1 and anything else for later.
const FICR_CONFIG_PART: usize = 0x1000_0130;
const FICR_CONFIG_REVISION: usize = 0x1000_0134;
const UICR_APPROTECT: usize = 0x1000_1208;
const APPROTECT_DISABLE: usize = 0x4000_0558;

/// On a hardened nRF52833, lets the debug port stay open after the next pin
/// or power reset, if the UICR says it should. On revision 1 it does
/// nothing, since APPROTECT.DISABLE only exists on the later ones.
pub(crate) fn keep_debug_port_open() {
    // SAFETY: both are FICR words, readable on every nRF52833, with no side
    // effects. nrfx reads them the same way.
    let (part, revision) = unsafe {
        (
            ptr::read_volatile(FICR_CONFIG_PART as *const u32),
            ptr::read_volatile(FICR_CONFIG_REVISION as *const u32),
        )
    };
    if part != 0x0D || revision == 0 {
        return;
    }
    // SAFETY: UICR.APPROTECT is a readable flash word on every nRF52833, and
    // on a hardened one APPROTECT.DISABLE is a writable register whose only
    // effect is on debug access, which is the point.
    unsafe {
        let uicr = ptr::read_volatile(UICR_APPROTECT as *const u32);
        ptr::write_volatile(APPROTECT_DISABLE as *mut u32, uicr);
    }
}
