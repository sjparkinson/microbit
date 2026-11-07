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

const NVMC_READY: u32 = 0x4001_E400;
const NVMC_CONFIG: u32 = 0x4001_E504;
const NVMC_ERASEPAGE: u32 = 0x4001_E508;
const CONFIG_READ: u32 = 0;
const CONFIG_WRITE: u32 = 1;
const CONFIG_ERASE: u32 = 2;

const FICR_CODEPAGESIZE: u32 = 0x1000_0010;
const FICR_CODESIZE: u32 = 0x1000_0014;

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

/// Programs `segments` (address and bytes, all in flash) and leaves the chip
/// running them. `log` gets a line for each step.
pub fn program<L: Link>(
    dap: &mut Dap<L>,
    chip: Chip,
    segments: &[(u32, Vec<u8>)],
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

    let words = words(segments);
    if let Some(&(address, _)) = words.iter().find(|&&(address, _)| address >= size) {
        return Err(format!(
            "the image reaches {address:#x}, past the end of flash"
        ));
    }

    let mut erase: Vec<u32> = words
        .iter()
        .map(|&(address, _)| address & !(page - 1))
        .collect();
    erase.dedup();
    log(&format!("erasing {} pages", erase.len()));
    nvmc(dap, CONFIG_ERASE)?;
    let erases: Vec<(u32, u32)> = erase.iter().map(|&p| (NVMC_ERASEPAGE, p)).collect();
    dap.write_each_and_wait(&erases, NVMC_READY)?;

    log(&format!("writing {} bytes", 4 * words.len()));
    nvmc(dap, CONFIG_WRITE)?;
    dap.write_each_and_wait(&words, NVMC_READY)?;
    nvmc(dap, CONFIG_READ)?;

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
    if dap.read_ap(CTRL_AP, CTRL_AP_APPROTECTSTATUS)? & 1 == 1 {
        return Ok(());
    }
    log("access port protection is on, so erasing the whole chip through the CTRL-AP");
    dap.write_ap(CTRL_AP, CTRL_AP_ERASEALL, 1)?;
    let mut erased = false;
    for _ in 0..10_000 {
        if dap.read_ap(CTRL_AP, CTRL_AP_ERASEALLSTATUS)? == 0 {
            erased = true;
            break;
        }
    }
    if !erased {
        return Err("the CTRL-AP erase never finished".into());
    }
    dap.write_ap(CTRL_AP, CTRL_AP_RESET, 1)?;
    dap.write_ap(CTRL_AP, CTRL_AP_RESET, 0)?;
    dap.write_ap(CTRL_AP, CTRL_AP_ERASEALL, 0)?;
    dap.clear_errors()?;
    if dap.read_ap(CTRL_AP, CTRL_AP_APPROTECTSTATUS)? & 1 == 0 {
        return Err("access port protection is still on after an erase".into());
    }
    Ok(())
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

fn nvmc<L: Link>(dap: &mut Dap<L>, config: u32) -> Result<(), String> {
    dap.write_each_and_wait(&[(NVMC_CONFIG, config)], NVMC_READY)
}

fn verify<L: Link>(dap: &mut Dap<L>, words: &[(u32, u32)]) -> Result<(), String> {
    // Contiguous runs, so each is one block read.
    let mut start = 0;
    while start < words.len() {
        let mut end = start + 1;
        while end < words.len() && words[end].0 == words[end - 1].0 + 4 {
            end += 1;
        }
        let read = dap.read_block(words[start].0, end - start)?;
        for (&(address, want), got) in words[start..end].iter().zip(read) {
            if got != want {
                return Err(format!(
                    "flash at {address:#010x} reads {got:#010x} after writing {want:#010x}"
                ));
            }
        }
        start = end;
    }
    Ok(())
}

/// Lets go of the core and resets it, so the new firmware starts from
/// scratch with the debugger out of the way.
fn run<L: Link>(dap: &mut Dap<L>) -> Result<(), String> {
    dap.write32(DHCSR, DBGKEY)?;
    let _ = dap.write32(AIRCR, VECTKEY_SYSRESETREQ);
    dap.clear_errors()
}

/// The segments as sorted, word-aligned `(address, word)` pairs, padding
/// with 0xFF (erased flash) where a segment starts or ends mid-word.
fn words(segments: &[(u32, Vec<u8>)]) -> Vec<(u32, u32)> {
    let mut bytes = std::collections::BTreeMap::new();
    for (address, data) in segments {
        for (i, &byte) in data.iter().enumerate() {
            bytes.insert(address + u32::try_from(i).expect("fits"), byte);
        }
    }
    let mut words: Vec<(u32, u32)> = Vec::new();
    for (&address, &byte) in &bytes {
        let word = address & !3;
        let shift = 8 * (address & 3);
        match words.last_mut() {
            Some((at, value)) if *at == word => {
                *value = (*value & !(0xFF << shift)) | (u32::from(byte) << shift);
            }
            _ => words.push((word, !(0xFF << shift) | (u32::from(byte) << shift))),
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    fn image() -> Vec<(u32, Vec<u8>)> {
        vec![
            (0x0, (0..=255u8).cycle().take(3000).collect()),
            (0x2000, vec![0xAA, 0xBB, 0xCC]),
        ]
    }

    #[test]
    fn words_are_aligned_and_padded_with_erased_bytes() {
        let words = words(&[(0x101, vec![0x11, 0x22]), (0x108, vec![0x33])]);
        assert_eq!(words, [(0x100, 0xFF22_11FF), (0x108, 0xFFFF_FF33)]);
    }

    #[test]
    fn programs_a_v1_and_leaves_it_running() {
        let mut dap = Dap::new(Fake::nrf51());
        dap.connect(1_000_000).unwrap();
        let mut lines = Vec::new();
        program(&mut dap, Chip::Nrf51, &image(), &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        let fake = dap.into_link();
        assert_eq!(&fake.flash[..3000], &image()[0].1[..]);
        assert_eq!(&fake.flash[0x2000..0x2004], &[0xAA, 0xBB, 0xCC, 0xFF]);
        // Pages the image doesn't touch keep what they had.
        assert_eq!(fake.flash[0x1000], 0x5A);
        assert!(fake.running, "{lines:?}");
        assert!(lines.iter().any(|l| l == "erasing 4 pages"), "{lines:?}");
    }

    #[test]
    fn programs_a_v2() {
        let mut dap = Dap::new(Fake::nrf52(false));
        dap.connect(1_000_000).unwrap();
        program(&mut dap, Chip::Nrf52, &image(), &mut |_| {}).unwrap();
        let fake = dap.into_link();
        assert_eq!(&fake.flash[..3000], &image()[0].1[..]);
        assert!(fake.running);
    }

    #[test]
    fn erases_a_locked_v2_through_the_ctrl_ap() {
        let mut dap = Dap::new(Fake::nrf52(true));
        dap.connect(1_000_000).unwrap();
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
    fn refuses_the_wrong_chip() {
        let mut dap = Dap::new(Fake::nrf51());
        dap.connect(1_000_000).unwrap();
        let err = program(&mut dap, Chip::Nrf52, &image(), &mut |_| {}).unwrap_err();
        assert!(
            err.contains("CTRL-AP") || err.contains("not an nRF52"),
            "{err}"
        );
    }

    #[test]
    fn refuses_an_image_past_the_end_of_flash() {
        let mut dap = Dap::new(Fake::nrf51());
        dap.connect(1_000_000).unwrap();
        let err = program(&mut dap, Chip::Nrf51, &[(0x4_0000, vec![0])], &mut |_| {}).unwrap_err();
        assert!(err.contains("past the end"), "{err}");
    }

    #[test]
    fn catches_flash_that_doesnt_take_the_write() {
        let mut fake = Fake::nrf51();
        fake.stuck_bit = Some(0x10);
        let mut dap = Dap::new(fake);
        dap.connect(1_000_000).unwrap();
        let err = program(&mut dap, Chip::Nrf51, &image(), &mut |_| {}).unwrap_err();
        assert!(err.contains("0x00000010"), "{err}");
    }
}
