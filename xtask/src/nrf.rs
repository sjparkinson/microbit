//! Programming an nRF51 or nRF52 over SWD, through its NVMC (non-volatile
//! memory controller): halt the core, erase the pages the image touches,
//! write it a word at a time, read it all back, then reset and run.
//!
//! NVMC and FICR addresses are the same on the nRF51822 (nRF51 Series
//! Reference Manual v3.0, §6 and §8) and nRF52833 (Product Specification
//! v1.7, §4.3 NVMC and §4.4 FICR). The Cortex-M debug registers are from the
//! Armv6-M and Armv7-M architecture manuals (C1.6). The CTRL-AP is nRF52 only
//! (nRF52833 PS §5.1 "CTRL-AP").

use crate::dap::{Dap, Link};
use crate::elf::Segment;

const NVMC_READY: u32 = 0x4001_E400;
const NVMC_CONFIG: u32 = 0x4001_E504;
const NVMC_ERASEPAGE: u32 = 0x4001_E508;
const CONFIG_READ: u32 = 0;
const CONFIG_WRITE: u32 = 1;
const CONFIG_ERASE: u32 = 2;

const FICR_CODEPAGESIZE: u32 = 0x1000_0010;
const FICR_CODESIZE: u32 = 0x1000_0014;
/// Undocumented FICR words that nrfx reads to tell the nRF52833's hardened
/// revision apart: 0x0D, then 0 for revision 1 and anything else for later
/// ones (nrfx nrf52_erratas.h, `nrf52_configuration_249`).
const FICR_CONFIG_PART: u32 = 0x1000_0130;
const FICR_CONFIG_REVISION: u32 = 0x1000_0134;

/// UICR.APPROTECT on the nRF52833, and its HwDisabled value (PS, UICR).
const UICR_APPROTECT: u32 = 0x1000_1208;
const HW_DISABLED: u32 = 0x5A;

const CPUID: u32 = 0xE000_ED00;
const AIRCR: u32 = 0xE000_ED0C;
const DHCSR: u32 = 0xE000_EDF0;
const DEMCR: u32 = 0xE000_EDFC;

/// DHCSR: the write key, C_DEBUGEN, C_HALT, and S_HALT in the status half.
const DBGKEY: u32 = 0xA05F_0000;
const C_DEBUGEN: u32 = 1 << 0;
const C_HALT: u32 = 1 << 1;
const S_HALT: u32 = 1 << 17;
/// DEMCR: halt on the way out of reset.
const VC_CORERESET: u32 = 1 << 0;
/// AIRCR: the write key and SYSRESETREQ.
const VECTKEY_SYSRESETREQ: u32 = 0x05FA_0004;

/// CPUID's part number: Cortex-M0 on the v1, Cortex-M4 on the v2.
const PART_M0: u32 = 0xC20;
const PART_M4: u32 = 0xC24;

/// The nRF52's CTRL-AP, which can erase the chip when the MEM-AP is locked.
const CTRL_AP: u8 = 1;
const CTRL_AP_RESET: u8 = 0x00;
const CTRL_AP_ERASEALL: u8 = 0x04;
const CTRL_AP_ERASEALLSTATUS: u8 = 0x08;
const CTRL_AP_APPROTECTSTATUS: u8 = 0x0C;
const CTRL_AP_IDR: u8 = 0xFC;
/// What the CTRL-AP's IDR reads (nRF52833 PS §5.1.1).
const CTRL_AP_ID: u32 = 0x0288_0000;

/// Which chip the program is for, from the board the user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Nrf51,
    Nrf52,
}

/// Programs `segments` and leaves the chip running them. Anything outside
/// flash is refused. `log` gets a line for each step.
pub fn program<L: Link>(
    dap: &mut Dap<L>,
    chip: Chip,
    segments: &[Segment],
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if chip == Chip::Nrf52 {
        // A missing AP reads as zero, so check it's really there before
        // reading its status.
        let idr = dap.read_ap(CTRL_AP, CTRL_AP_IDR)?;
        if idr != CTRL_AP_ID {
            return Err(format!(
                "there's no nRF52 CTRL-AP (AP 1 reads IDR {idr:#010x}), so this isn't an nRF52"
            ));
        }
        unlock(dap, log)?;
    }

    halt(dap)?;
    let part = (dap.read32(CPUID)? >> 4) & 0xFFF;
    let found = match part {
        PART_M0 => Chip::Nrf51,
        PART_M4 => Chip::Nrf52,
        _ => {
            return Err(format!(
                "the target's CPUID part is {part:#x}, which isn't a micro:bit's"
            ));
        }
    };
    if found != chip {
        return Err(format!(
            "the target is an {}, not an {}",
            name(found),
            name(chip)
        ));
    }

    let page = dap.read32(FICR_CODEPAGESIZE)?;
    let pages = dap.read32(FICR_CODESIZE)?;
    let size = page
        .checked_mul(pages)
        .filter(|_| page.is_power_of_two())
        .ok_or_else(|| {
            format!("the FICR says {pages} pages of {page} bytes, which can't be right")
        })?;
    log(&format!(
        "{}: {} KB of flash in {} pages of {} bytes",
        name(chip),
        size / 1024,
        pages,
        page
    ));
    if chip == Chip::Nrf52 {
        keep_open(dap, log)?;
    }

    // Sorted, so the last word is the highest. Flash starts at 0, so that's
    // the only end to check. This is what keeps UICR segments out.
    let words = words(segments)?;
    if let Some(&(address, _)) = words.last().filter(|&&(address, _)| address >= size) {
        return Err(format!(
            "the image reaches {address:#x}, past the end of flash, and only flash gets programmed"
        ));
    }

    // The NVMC CONFIG writes ride along in the same packets as the erases
    // and writes they're for.
    let mut erases = vec![(NVMC_CONFIG, CONFIG_ERASE)];
    erases.extend(
        words
            .iter()
            .map(|&(address, _)| (NVMC_ERASEPAGE, address & !(page - 1))),
    );
    erases.dedup();
    log(&format!("erasing {} pages", erases.len() - 1));
    dap.write_each_and_wait(&erases, NVMC_READY)?;

    log(&format!("writing {} bytes", 4 * words.len()));
    let mut writes = vec![(NVMC_CONFIG, CONFIG_WRITE)];
    writes.extend(&words);
    writes.push((NVMC_CONFIG, CONFIG_READ));
    dap.write_each_and_wait(&writes, NVMC_READY)?;

    log("verifying");
    verify(dap, &words)?;

    log("resetting");
    run(dap)
}

fn name(chip: Chip) -> &'static str {
    match chip {
        Chip::Nrf51 => "nRF51",
        Chip::Nrf52 => "nRF52",
    }
}

/// On the nRF52, if access port protection is on, erases the whole chip
/// through the CTRL-AP, which is the only way back in.
fn unlock<L: Link>(dap: &mut Dap<L>, log: &mut dyn FnMut(&str)) -> Result<(), String> {
    // APPROTECTSTATUS bit 0 reads 1 when protection is off.
    let open = |dap: &mut Dap<L>| -> Result<bool, String> {
        Ok(dap.read_ap(CTRL_AP, CTRL_AP_APPROTECTSTATUS)? & 1 == 1)
    };
    if open(dap)? {
        return Ok(());
    }
    log("access port protection is on, so erasing the whole chip through the CTRL-AP");
    dap.write_ap(CTRL_AP, CTRL_AP_ERASEALL, 1)?;
    // ERASEALLSTATUS reads 0 once it's done; the probe polls for it.
    dap.wait_ap(CTRL_AP, CTRL_AP_ERASEALLSTATUS, 1, 0)
        .map_err(|e| format!("the CTRL-AP erase never finished: {e}"))?;
    dap.write_ap(CTRL_AP, CTRL_AP_ERASEALL, 0)?;
    // A hardened nRF52833 is open as soon as the erase is done, until the
    // next pin or power reset (PS, "Debug and trace"), so don't reset one
    // that's already open. An older one may only notice at a reset.
    if !open(dap)? {
        dap.write_ap(CTRL_AP, CTRL_AP_RESET, 1)?;
        dap.write_ap(CTRL_AP, CTRL_AP_RESET, 0)?;
        dap.clear_errors()?;
    }
    if !open(dap)? {
        return Err("access port protection is still on after an erase".into());
    }
    Ok(())
}

/// On an nRF52833 with the hardened access port protection (revision 2,
/// build code Bxx), sets UICR.APPROTECT to HwDisabled, so the access port
/// stays open after a pin or power reset. That's only half of it: the
/// firmware has to write APPROTECT.DISABLE as it boots too, which `board`
/// does. Without both, every flash after a replug erases the whole chip.
///
/// A revision 1 chip is left alone, since HwDisabled isn't a value its
/// UICR.APPROTECT knows.
fn keep_open<L: Link>(dap: &mut Dap<L>, log: &mut dyn FnMut(&str)) -> Result<(), String> {
    // nrfx's nrf52_configuration_249(), for the nRF52833.
    let part = dap.read32(FICR_CONFIG_PART)?;
    let revision = dap.read32(FICR_CONFIG_REVISION)?;
    if part != 0x0D || revision == 0 {
        return Ok(());
    }
    match dap.read32(UICR_APPROTECT)? {
        HW_DISABLED => Ok(()),
        u32::MAX => {
            log("hardened access port protection, so setting UICR.APPROTECT to HwDisabled");
            dap.write_each_and_wait(
                &[
                    (NVMC_CONFIG, CONFIG_WRITE),
                    (UICR_APPROTECT, HW_DISABLED),
                    (NVMC_CONFIG, CONFIG_READ),
                ],
                NVMC_READY,
            )?;
            match dap.read32(UICR_APPROTECT)? {
                HW_DISABLED => Ok(()),
                got => Err(format!(
                    "UICR.APPROTECT reads {got:#010x} after writing HwDisabled"
                )),
            }
        }
        // Not erased, so setting it means erasing the whole UICR. Flashing
        // still works, through an erase-all each time.
        other => {
            log(&format!(
                "UICR.APPROTECT reads {other:#010x}, so the access port will lock again \
                 on the next power-up and the next flash will erase the whole chip"
            ));
            Ok(())
        }
    }
}

/// Halts the core, then resets it with a vector catch so it stops before
/// running a single instruction, with every peripheral back at reset.
fn halt<L: Link>(dap: &mut Dap<L>) -> Result<(), String> {
    dap.write32(DHCSR, DBGKEY | C_HALT | C_DEBUGEN)?;
    wait_halted(dap)?;
    dap.write32(DEMCR, VC_CORERESET)?;
    // The reset can swallow this write's acknowledgement.
    let _ = dap.write32(AIRCR, VECTKEY_SYSRESETREQ);
    dap.clear_errors()?;
    wait_halted(dap)?;
    dap.write32(DEMCR, 0)
}

fn wait_halted<L: Link>(dap: &mut Dap<L>) -> Result<(), String> {
    for _ in 0..100 {
        if dap.read32(DHCSR).is_ok_and(|dhcsr| dhcsr & S_HALT != 0) {
            return Ok(());
        }
        dap.clear_errors()?;
    }
    Err("the core wouldn't halt".into())
}

fn verify<L: Link>(dap: &mut Dap<L>, words: &[(u32, u32)]) -> Result<(), String> {
    // Contiguous runs, so each is one block read.
    for run in words.chunk_by(|a, b| b.0 == a.0 + 4) {
        let read = dap.read_block(run[0].0, run.len())?;
        for (&(address, want), got) in run.iter().zip(read) {
            if got != want {
                return Err(format!(
                    "flash at {address:#010x} reads {got:#010x} after writing {want:#010x}"
                ));
            }
        }
    }
    Ok(())
}

/// Lets go of the core and resets it, so the new firmware starts from
/// scratch with the debugger out of the way. Also what flash() does after
/// a failure, so the chip isn't left halted.
pub fn run<L: Link>(dap: &mut Dap<L>) -> Result<(), String> {
    dap.write32(DHCSR, DBGKEY)?;
    let _ = dap.write32(AIRCR, VECTKEY_SYSRESETREQ);
    dap.clear_errors()
}

/// The segments as sorted, word-aligned `(address, word)` pairs, padding
/// with 0xFF (erased flash) where a segment starts or ends mid-word. Two
/// segments that share a word share its pair.
///
/// One pass, so the segments have to be in address order and not overlap,
/// which is how `elf::Elf::parse` hands them over. Anything else is refused.
fn words(segments: &[Segment]) -> Result<Vec<(u32, u32)>, String> {
    let mut words: Vec<(u32, u32)> = Vec::new();
    let mut next = 0u32;
    for Segment { address, data } in segments {
        if *address < next {
            return Err(format!(
                "the image's segments overlap or are out of order at {address:#x}"
            ));
        }
        for (address, &byte) in (*address..).zip(data) {
            let word = address & !3;
            let shift = 8 * (address & 3);
            match words.last_mut() {
                Some((at, value)) if *at == word => {
                    *value = (*value & !(0xFF << shift)) | (u32::from(byte) << shift);
                }
                _ => words.push((word, !(0xFF << shift) | (u32::from(byte) << shift))),
            }
        }
        next = address.saturating_add(u32::try_from(data.len()).unwrap_or(u32::MAX));
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    fn segment(address: u32, data: Vec<u8>) -> Segment {
        Segment { address, data }
    }

    fn image() -> Vec<Segment> {
        vec![
            segment(0x0, (0..=255u8).cycle().take(3000).collect()),
            segment(0x2000, vec![0xAA, 0xBB, 0xCC]),
        ]
    }

    fn connected(fake: Fake) -> Dap<Fake> {
        let mut dap = Dap::new(fake);
        dap.connect(1_000_000).unwrap();
        dap
    }

    #[test]
    fn words_are_aligned_and_padded_with_erased_bytes() {
        let words = words(&[segment(0x101, vec![0x11, 0x22]), segment(0x108, vec![0x33])]);
        assert_eq!(words, Ok(vec![(0x100, 0xFF22_11FF), (0x108, 0xFFFF_FF33)]));
    }

    /// How `words` used to do it: every byte into a map, then back out.
    fn words_by_map(segments: &[Segment]) -> Vec<(u32, u32)> {
        let mut bytes = std::collections::BTreeMap::new();
        for Segment { address, data } in segments {
            for (address, &byte) in (*address..).zip(data) {
                bytes.insert(address, byte);
            }
        }
        let mut words: Vec<(u32, u32)> = Vec::new();
        for (&address, &byte) in &bytes {
            let (word, shift) = (address & !3, 8 * (address & 3));
            match words.last_mut() {
                Some((at, value)) if *at == word => {
                    *value = (*value & !(0xFF << shift)) | (u32::from(byte) << shift);
                }
                _ => words.push((word, !(0xFF << shift) | (u32::from(byte) << shift))),
            }
        }
        words
    }

    #[test]
    fn words_match_the_byte_map_they_replaced() {
        let cases = [
            image(),
            vec![],
            vec![segment(0x3, vec![1])],
            // Two segments sharing a word, from both ends.
            vec![
                segment(0x10, vec![1, 2, 3, 4, 5]),
                segment(0x16, vec![6, 7]),
            ],
            vec![segment(0x10, vec![1, 2]), segment(0x12, vec![3, 4])],
            // An empty segment in the middle, and a gap of whole words.
            vec![
                segment(0x20, vec![9; 7]),
                segment(0x27, vec![]),
                segment(0x40, vec![8; 9]),
            ],
        ];
        for segments in cases {
            assert_eq!(
                words(&segments),
                Ok(words_by_map(&segments)),
                "{segments:?}"
            );
        }
    }

    #[test]
    fn words_refuse_overlapping_or_unsorted_segments() {
        let overlap = [segment(0x10, vec![1; 8]), segment(0x14, vec![2])];
        assert!(words(&overlap).unwrap_err().contains("overlap"));
        let unsorted = [segment(0x100, vec![1]), segment(0x0, vec![2])];
        assert!(words(&unsorted).unwrap_err().contains("out of order"));
    }

    #[test]
    fn programs_a_v1_and_leaves_it_running() {
        let mut dap = connected(Fake::nrf51());
        let mut lines = Vec::new();
        program(&mut dap, Chip::Nrf51, &image(), &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        let fake = dap.into_link();
        assert_eq!(&fake.flash[..3000], &image()[0].data[..]);
        assert_eq!(&fake.flash[0x2000..0x2004], &[0xAA, 0xBB, 0xCC, 0xFF]);
        // Pages the image doesn't touch keep what they had.
        assert_eq!(fake.flash[0x1000], 0x5A);
        assert!(fake.running, "{lines:?}");
        assert!(lines.iter().any(|l| l == "erasing 4 pages"), "{lines:?}");
    }

    #[test]
    fn programs_a_v2() {
        let mut dap = connected(Fake::nrf52(false));
        program(&mut dap, Chip::Nrf52, &image(), &mut |_| {}).unwrap();
        let fake = dap.into_link();
        assert_eq!(&fake.flash[..3000], &image()[0].data[..]);
        assert!(fake.running);
    }

    #[test]
    fn erases_a_locked_v2_through_the_ctrl_ap() {
        let mut dap = connected(Fake::nrf52(true));
        let mut lines = Vec::new();
        program(&mut dap, Chip::Nrf52, &image(), &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert!(lines[0].contains("protection is on"), "{lines:?}");
        let fake = dap.into_link();
        // The erase-all wiped the page the image doesn't touch.
        assert_eq!(fake.flash[0x1000 * 4], 0xFF);
        assert!(fake.running);
    }

    #[test]
    fn leaves_a_revision_1_uicr_alone() {
        let mut dap = connected(Fake::nrf52(true));
        program(&mut dap, Chip::Nrf52, &image(), &mut |_| {}).unwrap();
        let fake = dap.into_link();
        // It needed the CTRL-AP reset to open, and HwDisabled means nothing
        // to it.
        assert_eq!(fake.ctrl_ap_resets, 1);
        assert_eq!(fake.uicr_approtect, u32::MAX);
    }

    #[test]
    fn opens_a_hardened_v2_and_keeps_it_open() {
        let mut fake = Fake::nrf52_hardened();
        fake.firmware_disables_approtect = true;
        let mut dap = connected(fake);
        let mut lines = Vec::new();
        program(&mut dap, Chip::Nrf52, &image(), &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert!(lines[0].contains("protection is on"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("HwDisabled")), "{lines:?}");
        let mut fake = dap.into_link();
        // Open straight after the erase, so no CTRL-AP reset to lock it.
        assert_eq!(fake.ctrl_ap_resets, 0);
        assert_eq!(fake.uicr_approtect, 0x5A);
        assert!(fake.running);

        // Unplugged and plugged back in, it's still open, so the next flash
        // only erases the pages it needs.
        fake.power_cycle();
        let mut dap = connected(fake);
        let mut lines = Vec::new();
        program(&mut dap, Chip::Nrf52, &image(), &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("protection is on") || l.contains("HwDisabled")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_hardened_v2_locks_again_without_the_firmwares_half() {
        let mut dap = connected(Fake::nrf52_hardened());
        program(&mut dap, Chip::Nrf52, &image(), &mut |_| {}).unwrap();
        let mut fake = dap.into_link();
        fake.power_cycle();
        let mut dap = connected(fake);
        let mut lines = Vec::new();
        program(&mut dap, Chip::Nrf52, &image(), &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert!(lines[0].contains("protection is on"), "{lines:?}");
    }

    #[test]
    fn refuses_the_wrong_chip() {
        let mut dap = connected(Fake::nrf51());
        let err = program(&mut dap, Chip::Nrf52, &image(), &mut |_| {}).unwrap_err();
        assert!(
            err.contains("CTRL-AP") || err.contains("not an nRF52"),
            "{err}"
        );
    }

    #[test]
    fn refuses_an_image_past_the_end_of_flash() {
        let mut dap = connected(Fake::nrf51());
        let err = program(
            &mut dap,
            Chip::Nrf51,
            &[segment(0x4_0000, vec![0])],
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.contains("past the end"), "{err}");
    }

    #[test]
    fn refuses_uicr_and_leaves_flash_alone() {
        let mut dap = connected(Fake::nrf52(false));
        let mut segments = image();
        segments.push(segment(0x1000_1014, vec![0xAA; 4]));
        let err = program(&mut dap, Chip::Nrf52, &segments, &mut |_| {}).unwrap_err();
        assert!(err.contains("0x10001014"), "{err}");
        assert!(dap.into_link().flash.iter().all(|&b| b == 0x5A));
    }

    #[test]
    fn catches_flash_that_doesnt_take_the_write() {
        let mut fake = Fake::nrf51();
        fake.stuck_word = Some(0x10);
        let mut dap = connected(fake);
        let err = program(&mut dap, Chip::Nrf51, &image(), &mut |_| {}).unwrap_err();
        assert!(err.contains("0x00000010"), "{err}");
    }
}
