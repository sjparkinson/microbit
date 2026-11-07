//! Just enough ELF32 to size a firmware image and pull out what gets
//! programmed: the section headers for RAM, the program headers for flash.

use std::ops::Range;

const PT_LOAD: u32 = 1;
const SHF_ALLOC: u32 = 0x2;
const EM_ARM: u16 = 40;

#[derive(Debug)]
pub struct Elf {
    segments: Vec<Segment>,
    sections: Vec<Section>,
}

/// A loadable piece of the image at its load (physical) address. For .data
/// that's its initial values in flash, not where cortex-m-rt copies it to.
#[derive(Debug, PartialEq, Eq)]
pub struct Segment {
    pub address: u32,
    pub data: Vec<u8>,
}

#[derive(Debug)]
struct Section {
    flags: u32,
    addr: u32,
    size: u32,
}

impl Elf {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let r = Reader(bytes);
        if bytes.get(..4) != Some(b"\x7fELF".as_slice()) {
            return Err("not an ELF file".into());
        }
        if r.u8(4)? != 1 || r.u8(5)? != 1 {
            return Err("not a 32-bit little-endian ELF file".into());
        }
        if r.u16(18)? != EM_ARM {
            return Err("not an Arm ELF file".into());
        }

        let (phoff, phentsize, phnum) = (r.u32(28)?, r.u16(42)?, r.u16(44)?);
        let mut segments = Vec::new();
        for i in 0..usize::from(phnum) {
            let at = phoff as usize + i * usize::from(phentsize);
            let (kind, offset, paddr, filesz) =
                (r.u32(at)?, r.u32(at + 4)?, r.u32(at + 12)?, r.u32(at + 16)?);
            // .bss and the stack are PT_LOAD too, but with nothing in the
            // file, so nothing to program.
            if kind == PT_LOAD && filesz > 0 {
                let data = r.slice(offset as usize, filesz as usize)?.to_vec();
                segments.push(Segment {
                    address: paddr,
                    data,
                });
            }
        }
        segments.sort_by_key(|s| s.address);

        let (shoff, shentsize, shnum) = (r.u32(32)?, r.u16(46)?, r.u16(48)?);
        let mut sections = Vec::new();
        for i in 0..usize::from(shnum) {
            let at = shoff as usize + i * usize::from(shentsize);
            sections.push(Section {
                flags: r.u32(at + 8)?,
                addr: r.u32(at + 12)?,
                size: r.u32(at + 20)?,
            });
        }

        Ok(Self { segments, sections })
    }

    /// What gets programmed, in address order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Bytes programmed into `flash`: .vector_table, .text, .rodata and
    /// .data's initial values. Anything outside it (UICR, say) isn't counted.
    pub fn flash_bytes(&self, flash: &Range<u32>) -> u32 {
        self.segments
            .iter()
            .filter(|s| flash.contains(&s.address))
            .map(|s| s.data.len() as u32)
            .sum()
    }

    /// Bytes of `ram` the image reserves: every allocated section that lives
    /// there, so .data, .bss and .uninit. The stack isn't a section, so it
    /// isn't counted: it grows down into whatever this leaves.
    pub fn ram_bytes(&self, ram: &Range<u32>) -> u32 {
        self.sections
            .iter()
            .filter(|s| s.flags & SHF_ALLOC != 0 && ram.contains(&s.addr))
            .map(|s| s.size)
            .sum()
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn slice(&self, at: usize, len: usize) -> Result<&[u8], String> {
        at.checked_add(len)
            .and_then(|end| self.0.get(at..end))
            .ok_or_else(|| format!("truncated: wanted {len} bytes at offset {at:#x}"))
    }

    fn u8(&self, at: usize) -> Result<u8, String> {
        Ok(self.slice(at, 1)?[0])
    }

    fn u16(&self, at: usize) -> Result<u16, String> {
        let b = self.slice(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        let b = self.slice(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Builds a minimal ELF32 Arm file by hand: a header, then the program
    /// headers, then the section headers, then the segment bytes. Each
    /// segment is (paddr, file bytes, memory size); each section is
    /// (flags, addr, size).
    pub fn fixture(segments: &[(u32, &[u8], u32)], sections: &[(u32, u32, u32)]) -> Vec<u8> {
        let phoff = 52u32;
        let shoff = phoff + 32 * segments.len() as u32;
        let mut data_at = shoff + 40 * sections.len() as u32;

        let mut out = Vec::new();
        out.extend(b"\x7fELF");
        out.extend([1, 1, 1, 0]); // ELFCLASS32, little-endian, version 1
        out.extend([0; 8]);
        out.extend(2u16.to_le_bytes()); // ET_EXEC
        out.extend(EM_ARM.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(0u32.to_le_bytes()); // entry
        out.extend(phoff.to_le_bytes());
        out.extend(shoff.to_le_bytes());
        out.extend(0x0500_0400u32.to_le_bytes()); // EABI5, hard float
        out.extend(52u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((segments.len() as u16).to_le_bytes());
        out.extend(40u16.to_le_bytes());
        out.extend((sections.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes()); // no section name table needed
        assert_eq!(out.len(), 52);

        for (paddr, bytes, memsz) in segments {
            let filesz = bytes.len() as u32;
            for word in [PT_LOAD, data_at, *paddr, *paddr, filesz, *memsz, 5, 4] {
                out.extend(word.to_le_bytes());
            }
            data_at += filesz;
        }
        for (flags, addr, size) in sections {
            for word in [0, 1, *flags, *addr, 0, *size, 0, 0, 4, 0] {
                out.extend(word.to_le_bytes());
            }
        }
        for (_, bytes, _) in segments {
            out.extend(*bytes);
        }
        out
    }

    const FLASH: Range<u32> = 0..0x8_0000;
    const RAM: Range<u32> = 0x2000_0000..0x2002_0000;

    #[test]
    fn sizes_flash_from_segments_and_ram_from_sections() {
        let elf = Elf::parse(&fixture(
            &[
                // .vector_table and .text
                (0x0000, &[1, 2, 3, 4, 5, 6, 7, 8], 8),
                // .data's initial values, loaded after .text
                (0x0008, &[9, 9], 2),
                // .bss: in memory, not in the file
                (0x2000_0002, &[], 16),
            ],
            &[
                (SHF_ALLOC, 0x0000, 8),       // .text
                (SHF_ALLOC, 0x2000_0000, 2),  // .data
                (SHF_ALLOC, 0x2000_0002, 16), // .bss
                (0, 0x2000_0000, 100),        // debug info, not allocated
            ],
        ))
        .unwrap();

        assert_eq!(elf.flash_bytes(&FLASH), 10);
        assert_eq!(elf.ram_bytes(&RAM), 18);
        assert_eq!(
            elf.segments(),
            [
                Segment {
                    address: 0,
                    data: vec![1, 2, 3, 4, 5, 6, 7, 8]
                },
                Segment {
                    address: 8,
                    data: vec![9, 9]
                },
            ]
        );
    }

    #[test]
    fn leaves_uicr_out_of_the_flash_count_but_programs_it() {
        let elf = Elf::parse(&fixture(
            &[(0x0000, &[0; 4], 4), (0x1000_1014, &[0xAA; 4], 4)],
            &[],
        ))
        .unwrap();
        assert_eq!(elf.flash_bytes(&FLASH), 4);
        assert_eq!(elf.segments().len(), 2);
    }

    #[test]
    fn puts_segments_in_address_order() {
        let elf = Elf::parse(&fixture(&[(0x100, &[2], 1), (0x000, &[1], 1)], &[])).unwrap();
        let addresses: Vec<u32> = elf.segments().iter().map(|s| s.address).collect();
        assert_eq!(addresses, [0x000, 0x100]);
    }

    #[test]
    fn refuses_what_isnt_a_32_bit_arm_elf() {
        assert!(
            Elf::parse(b"MZ\x90\x00")
                .unwrap_err()
                .contains("not an ELF")
        );

        let mut avr = fixture(&[], &[]);
        avr[18] = 83;
        assert!(Elf::parse(&avr).unwrap_err().contains("not an Arm"));

        let mut elf64 = fixture(&[], &[]);
        elf64[4] = 2;
        assert!(Elf::parse(&elf64).unwrap_err().contains("32-bit"));
    }

    #[test]
    fn reports_truncation_instead_of_panicking() {
        let mut bytes = fixture(&[(0, &[1, 2, 3, 4], 4)], &[]);
        bytes.truncate(bytes.len() - 2);
        assert!(Elf::parse(&bytes).unwrap_err().contains("truncated"));
    }
}
