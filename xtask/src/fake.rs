//! A pretend micro:bit on the far end of a CMSIS-DAP link, for the tests: a
//! DAPLink that answers commands, an SWD debug port, a MEM-AP onto a small
//! model of the nRF's memory map, and the nRF52's CTRL-AP.
//!
//! It's my reading of the same documents dap.rs and nrf.rs were written
//! from, so it finds slips and proves nothing about a real chip. It's strict
//! where that's cheap: flash written without NVMC CONFIG set to write is a
//! FAULT, flash bits only ever clear on a write, and any address it doesn't
//! model is a FAULT.

const PAGE_51: usize = 1024;
const PAGE_52: usize = 4096;

pub struct Fake {
    pub flash: Vec<u8>,
    page: usize,
    nrf52: bool,
    locked: bool,
    /// A flash word that ignores writes, to check the verify.
    pub stuck_word: Option<u32>,
    pub running: bool,
    /// Answer the next DAP_Transfer with WAIT before doing any of it.
    pub wait_next_transfer: bool,
    powered: bool,
    sticky: bool,
    select: u32,
    csw: u32,
    tar: u32,
    mask: u32,
    config: u32,
    halted: bool,
    debugen: bool,
    demcr: u32,
    erase_all: bool,
}

const OK: u8 = 0x01;
const WAIT: u8 = 0x02;
const FAULT: u8 = 0x04;
const MISMATCH: u8 = 0x10;

impl Fake {
    pub fn nrf51() -> Self {
        Self::new(256 * 1024, PAGE_51, false, false)
    }

    pub fn nrf52(locked: bool) -> Self {
        Self::new(512 * 1024, PAGE_52, true, locked)
    }

    fn new(size: usize, page: usize, nrf52: bool, locked: bool) -> Self {
        Fake {
            // Whatever was there before: not erased.
            flash: vec![0x5A; size],
            page,
            nrf52,
            locked,
            stuck_word: None,
            running: true,
            wait_next_transfer: false,
            powered: false,
            sticky: false,
            select: 0,
            csw: 0,
            tar: 0,
            mask: u32::MAX,
            config: 0,
            halted: false,
            debugen: false,
            demcr: 0,
            erase_all: false,
        }
    }

    fn read_mem(&mut self, address: u32) -> Option<u32> {
        let a = address as usize;
        Some(match address {
            _ if a + 4 <= self.flash.len() && address.is_multiple_of(4) => {
                u32::from_le_bytes(self.flash[a..a + 4].try_into().ok()?)
            }
            0x1000_0010 => self.page as u32,
            0x1000_0014 => (self.flash.len() / self.page) as u32,
            0x4001_E400 => 1,
            0x4001_E504 => self.config,
            0xE000_ED00 if self.nrf52 => 0x410F_C241,
            0xE000_ED00 => 0x410C_C200,
            0xE000_EDF0 => (u32::from(self.halted) << 17) | u32::from(self.debugen),
            0xE000_EDFC => self.demcr,
            _ => return None,
        })
    }

    fn write_mem(&mut self, address: u32, value: u32) -> Option<()> {
        let a = address as usize;
        match address {
            _ if a + 4 <= self.flash.len() && address.is_multiple_of(4) => {
                if self.config != 1 {
                    return None;
                }
                if Some(address) == self.stuck_word {
                    return Some(());
                }
                for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
                    self.flash[a + i] &= byte;
                }
            }
            0x4001_E504 if value <= 2 => self.config = value,
            0x4001_E508
                if self.config == 2
                    && (value as usize).is_multiple_of(self.page)
                    && (value as usize) < self.flash.len() =>
            {
                let page = value as usize;
                self.flash[page..page + self.page].fill(0xFF);
            }
            0xE000_EDF0 if value >> 16 == 0xA05F => {
                self.debugen = value & 1 != 0;
                self.halted = self.debugen && value & 2 != 0;
                if self.halted {
                    self.running = false;
                }
            }
            0xE000_EDFC => self.demcr = value,
            0xE000_ED0C if value == 0x05FA_0004 => {
                self.config = 0;
                self.halted = self.debugen && self.demcr & 1 != 0;
                self.running = !self.halted && !self.debugen;
            }
            _ => return None,
        }
        Some(())
    }

    /// One AP or DP access: `Some(value)` for a read, `None` for a FAULT.
    fn access(&mut self, request: u8, data: u32) -> Option<u32> {
        let ap = request & 1 != 0;
        let read = request & 2 != 0;
        let a = request & 0x0C;
        if !ap {
            return match (read, a) {
                (true, 0x0) => Some(if self.nrf52 { 0x2BA0_1477 } else { 0x0BB1_1477 }),
                (false, 0x0) => {
                    self.sticky = false;
                    Some(0)
                }
                (true, 0x4) => Some(if self.powered {
                    0xA000_0000 | 0x5000_0000
                } else {
                    0
                }),
                (false, 0x4) => {
                    self.powered = data & 0x5000_0000 == 0x5000_0000;
                    Some(0)
                }
                (false, 0x8) => {
                    self.select = data;
                    Some(0)
                }
                _ => None,
            };
        }
        if self.sticky || !self.powered {
            return None;
        }
        let apsel = self.select >> 24;
        let reg = (self.select & 0xF0) as u8 | a;
        match apsel {
            0 => self.mem_ap(read, reg, data),
            1 if self.nrf52 => self.ctrl_ap(read, reg, data),
            // An AP that isn't there reads as zero.
            _ => Some(0),
        }
    }

    fn mem_ap(&mut self, read: bool, reg: u8, data: u32) -> Option<u32> {
        match (read, reg) {
            (_, 0x0C) if self.locked => None,
            (true, 0x00) => Some(self.csw),
            (false, 0x00) => {
                self.csw = data;
                Some(0)
            }
            (true, 0x04) => Some(self.tar),
            (false, 0x04) => {
                self.tar = data;
                Some(0)
            }
            (true, 0x0C) => {
                let value = self.read_mem(self.tar)?;
                self.increment();
                Some(value)
            }
            (false, 0x0C) => {
                self.write_mem(self.tar, data)?;
                self.increment();
                Some(0)
            }
            (true, 0xFC) => Some(0x0477_0021),
            _ => None,
        }
    }

    fn increment(&mut self) {
        if self.csw & 0x30 == 0x10 {
            // Within a 1 KB block only.
            self.tar = (self.tar & !0x3FF) | (self.tar.wrapping_add(4) & 0x3FF);
        }
    }

    fn ctrl_ap(&mut self, read: bool, reg: u8, data: u32) -> Option<u32> {
        match (read, reg) {
            (false, 0x00) => Some(0),
            (false, 0x04) => {
                if data & 1 != 0 && !self.erase_all {
                    self.flash.fill(0xFF);
                    self.locked = false;
                }
                self.erase_all = data & 1 != 0;
                Some(0)
            }
            (true, 0x08) => Some(0),
            (true, 0x0C) => Some(u32::from(!self.locked)),
            (true, 0xFC) => Some(0x0288_0000),
            _ => None,
        }
    }

    fn transfer(&mut self, command: &[u8]) -> Vec<u8> {
        if std::mem::take(&mut self.wait_next_transfer) {
            return vec![0x05, 0, WAIT];
        }
        let count = command[2];
        let mut at = 3;
        let mut done = 0u8;
        let mut ack = OK;
        let mut reads = Vec::new();
        for _ in 0..count {
            let request = command[at];
            at += 1;
            let has_data = request & 0x20 != 0 || request & 0x10 != 0 || request & 2 == 0;
            let data = if has_data {
                let word = u32::from_le_bytes(command[at..at + 4].try_into().unwrap());
                at += 4;
                word
            } else {
                0
            };
            // Counted, as DAPLink does.
            if request & 0x20 != 0 {
                self.mask = data;
                done += 1;
                continue;
            }
            match self.access(request, data) {
                None => {
                    self.sticky = true;
                    ack = FAULT;
                    break;
                }
                Some(value) if request & 0x10 != 0 => {
                    if value & self.mask != data {
                        ack = MISMATCH;
                        break;
                    }
                }
                Some(value) if request & 2 != 0 => reads.extend(value.to_le_bytes()),
                Some(_) => {}
            }
            done += 1;
        }
        let mut response = vec![0x05, done, ack];
        response.extend(reads);
        response
    }
}

impl crate::dap::Link for Fake {
    fn exchange(&mut self, command: &[u8]) -> Result<Vec<u8>, String> {
        assert!(command.len() <= 64, "a command bigger than a HID report");
        let response = match command[0] {
            0x00 => match command[1] {
                0xFF => vec![0x00, 2, 64, 0],
                _ => vec![0x00, 0],
            },
            0x02 => vec![0x02, 1],
            0x05 => self.transfer(command),
            0x03 | 0x04 | 0x11 | 0x12 | 0x13 => vec![command[0], 0],
            other => panic!("the fake doesn't know command {other:#04x}"),
        };
        assert!(response.len() <= 64, "a response bigger than a HID report");
        Ok(response)
    }
}
