//! CMSIS-DAP, the protocol the micro:bit's interface chip (DAPLink) speaks on
//! its HID interface, and Arm debug (ADIv5) register and memory access over
//! SWD on top of it.
//!
//! Command numbers and packet layouts are from Arm's CMSIS-DAP reference
//! (arm-software.github.io/CMSIS_5/DAP/html), and the DP and MEM-AP registers
//! from the ADIv5 architecture spec (IHI 0031).

/// One way to send a command packet and get its response back: USB HID on a
/// real probe (hid.rs), or a pretend chip in the tests (fake.rs).
pub trait Link {
    fn exchange(&mut self, command: &[u8]) -> Result<Vec<u8>, String>;
}

const INFO: u8 = 0x00;
const CONNECT: u8 = 0x02;
const DISCONNECT: u8 = 0x03;
const TRANSFER_CONFIGURE: u8 = 0x04;
const TRANSFER: u8 = 0x05;
const SWJ_CLOCK: u8 = 0x11;
const SWJ_SEQUENCE: u8 = 0x12;
const SWD_CONFIGURE: u8 = 0x13;

/// DAP_Info ID for the largest packet the probe takes.
const INFO_PACKET_SIZE: u8 = 0xFF;

/// Transfer request bits.
const REQ_AP: u8 = 0x01;
const REQ_READ: u8 = 0x02;
const REQ_MATCH: u8 = 0x10;
const REQ_MATCH_MASK: u8 = 0x20;

/// Transfer response: the ACK in the low three bits, then error flags.
const ACK_OK: u8 = 0x01;
const ACK_WAIT: u8 = 0x02;
const ACK_FAULT: u8 = 0x04;
const RESP_PROTOCOL_ERROR: u8 = 0x08;
const RESP_MISMATCH: u8 = 0x10;

/// DP registers.
const DP_IDCODE: u8 = 0x0;
const DP_ABORT: u8 = 0x0;
const DP_CTRL_STAT: u8 = 0x4;
const DP_SELECT: u8 = 0x8;

/// CTRL/STAT power-up requests and their acknowledgements.
const CSYSPWRUPREQ: u32 = 1 << 30;
const CDBGPWRUPREQ: u32 = 1 << 28;
const CSYSPWRUPACK: u32 = 1 << 31;
const CDBGPWRUPACK: u32 = 1 << 29;
/// ABORT: clear every sticky error flag.
const ABORT_CLEAR_ALL: u32 = 0x1E;

/// MEM-AP registers.
const AP_CSW: u8 = 0x00;
const AP_TAR: u8 = 0x04;
const AP_DRW: u8 = 0x0C;
/// CSW: 32-bit accesses, with or without auto-increment. 0x23000000 sets the
/// HPROT and master-type bits the way pyOCD and OpenOCD do for a Cortex-M.
const CSW_WORD: u32 = 0x2300_0002;
const CSW_INCREMENT: u32 = 0x10;

/// The MEM-AP the Cortex-M's memory is behind, on both chips.
const MEM_AP: u8 = 0;

/// TAR auto-increment is only guaranteed within a 1 KB block (ADIv5 §7.2.2).
const INCREMENT_WRAP: u32 = 1024;

/// One DAP_Transfer request: the request byte and the word it sends, if any.
#[derive(Clone, Copy)]
struct Op {
    request: u8,
    data: Option<u32>,
}

impl Op {
    fn write(request: u8, data: u32) -> Self {
        Op {
            request,
            data: Some(data),
        }
    }

    fn read(request: u8) -> Self {
        Op {
            request: request | REQ_READ,
            data: None,
        }
    }

    fn len(&self) -> usize {
        1 + if self.data.is_some() { 4 } else { 0 }
    }

    fn reads(&self) -> bool {
        self.request & REQ_READ != 0 && self.request & REQ_MATCH == 0
    }
}

pub struct Dap<L: Link> {
    link: L,
    packet: usize,
    select: Option<u32>,
    csw: Option<u32>,
}

impl<L: Link> Dap<L> {
    pub fn new(link: L) -> Self {
        Dap {
            link,
            packet: 64,
            select: None,
            csw: None,
        }
    }

    #[cfg(test)]
    pub fn into_link(self) -> L {
        self.link
    }

    fn command(&mut self, command: &[u8]) -> Result<Vec<u8>, String> {
        let response = self.link.exchange(command)?;
        if response.first() != command.first() {
            return Err(format!(
                "the probe answered command {:#04x} with {:02x?}",
                command[0],
                &response[..response.len().min(4)]
            ));
        }
        Ok(response)
    }

    /// A command whose response is one status byte, 0 for OK.
    fn status(&mut self, command: &[u8]) -> Result<(), String> {
        match self.command(command)?.get(1) {
            Some(0) => Ok(()),
            other => Err(format!(
                "the probe refused command {:#04x} ({other:?})",
                command[0]
            )),
        }
    }

    fn info(&mut self, id: u8) -> Result<Vec<u8>, String> {
        let response = self.command(&[INFO, id])?;
        let len = usize::from(*response.get(1).ok_or("a DAP_Info with no length")?);
        response
            .get(2..2 + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "a DAP_Info shorter than it says".into())
    }

    /// Connects over SWD, switches the target's debug port from JTAG to SWD,
    /// powers up its debug domain and returns its IDCODE.
    pub fn connect(&mut self, clock_hz: u32) -> Result<u32, String> {
        if let [lo, hi] = self.info(INFO_PACKET_SIZE)?[..] {
            self.packet = usize::from(u16::from_le_bytes([lo, hi])).clamp(16, 64);
        }
        let response = self.command(&[CONNECT, 1])?;
        if response.get(1) != Some(&1) {
            return Err("the probe wouldn't connect in SWD mode".into());
        }
        let mut clock = vec![SWJ_CLOCK];
        clock.extend(clock_hz.to_le_bytes());
        self.status(&clock)?;
        // No idle cycles, and generous retries: a page erase keeps NVMC READY
        // low for up to 85 ms on the nRF52, and every retry is one SWD read.
        let mut configure = vec![TRANSFER_CONFIGURE, 0];
        configure.extend(64u16.to_le_bytes());
        configure.extend(u16::MAX.to_le_bytes());
        self.status(&configure)?;
        self.status(&[SWD_CONFIGURE, 0])?;

        // Line reset, the JTAG-to-SWD switch (0xE79E, LSB first), another
        // line reset, then idle (ADIv5 §B4.3.2).
        self.sequence(56, &[0xFF; 7])?;
        self.sequence(16, &[0x9E, 0xE7])?;
        self.sequence(56, &[0xFF; 7])?;
        self.sequence(8, &[0x00])?;

        let idcode = self.read_dp(DP_IDCODE)?;
        self.write_dp(DP_ABORT, ABORT_CLEAR_ALL)?;
        self.write_dp(DP_SELECT, 0)?;
        self.select = Some(0);
        self.csw = None;
        self.write_dp(DP_CTRL_STAT, CSYSPWRUPREQ | CDBGPWRUPREQ)?;
        let acks = CSYSPWRUPACK | CDBGPWRUPACK;
        for _ in 0..100 {
            if self.read_dp(DP_CTRL_STAT)? & acks == acks {
                return Ok(idcode);
            }
        }
        Err("the target's debug port never powered up".into())
    }

    pub fn disconnect(&mut self) -> Result<(), String> {
        self.status(&[DISCONNECT])
    }

    fn sequence(&mut self, bits: u8, data: &[u8]) -> Result<(), String> {
        let mut command = vec![SWJ_SEQUENCE, bits];
        command.extend(data);
        self.status(&command)
    }

    /// Clears the sticky errors a FAULT leaves behind, so the next transfer
    /// can go through.
    pub fn clear_errors(&mut self) -> Result<(), String> {
        self.write_dp(DP_ABORT, ABORT_CLEAR_ALL)
    }

    /// Runs `ops` as one DAP_Transfer and returns the words read.
    fn transfer(&mut self, ops: &[Op]) -> Result<Vec<u32>, String> {
        let result = self.transfer_once(ops);
        // `select` counts a SELECT write as done before it's sent, and a
        // failed transfer may have stopped short of it, so forget it.
        if result.is_err() {
            self.select = None;
        }
        result
    }

    fn transfer_once(&mut self, ops: &[Op]) -> Result<Vec<u32>, String> {
        let mut command = vec![TRANSFER, 0, u8::try_from(ops.len()).expect("few ops")];
        for op in ops {
            command.push(op.request);
            if let Some(data) = op.data {
                command.extend(data.to_le_bytes());
            }
        }
        debug_assert!(command.len() <= self.packet, "a transfer packet too big");
        let response = self.command(&command)?;
        let (done, ack) = match response[..] {
            [_, done, ack, ..] => (usize::from(done), ack),
            _ => return Err("a DAP_Transfer response with no status".into()),
        };
        // DAPLink counts a match-mask write as a transfer too, even though
        // nothing goes over SWD for it. Seen on a v1's 0253.
        let expected = ops.len();
        if done != expected || ack != ACK_OK {
            return Err(describe(ack, done, expected));
        }
        let reads = ops.iter().filter(|op| op.reads()).count();
        let words: Vec<u32> = response[3..]
            .as_chunks::<4>()
            .0
            .iter()
            .take(reads)
            .map(|&word| u32::from_le_bytes(word))
            .collect();
        if words.len() == reads {
            Ok(words)
        } else {
            Err("a DAP_Transfer response shorter than its reads".into())
        }
    }

    fn read_dp(&mut self, reg: u8) -> Result<u32, String> {
        Ok(self.transfer(&[Op::read(reg)])?[0])
    }

    fn write_dp(&mut self, reg: u8, value: u32) -> Result<(), String> {
        self.transfer(&[Op::write(reg, value)]).map(drop)
    }

    /// The SELECT write `ap`'s register `reg` needs, if it isn't selected.
    fn select(&mut self, ap: u8, reg: u8) -> Option<Op> {
        let select = (u32::from(ap) << 24) | u32::from(reg & 0xF0);
        if self.select == Some(select) {
            return None;
        }
        self.select = Some(select);
        Some(Op::write(DP_SELECT, select))
    }

    fn ap_ops(&mut self, ap: u8, reg: u8, op: Op) -> Vec<Op> {
        let mut ops: Vec<Op> = self.select(ap, reg).into_iter().collect();
        ops.push(Op {
            request: op.request | REQ_AP | (reg & 0x0C),
            data: op.data,
        });
        ops
    }

    pub fn read_ap(&mut self, ap: u8, reg: u8) -> Result<u32, String> {
        let ops = self.ap_ops(ap, reg, Op::read(0));
        Ok(*self.transfer(&ops)?.last().expect("one read"))
    }

    pub fn write_ap(&mut self, ap: u8, reg: u8, value: u32) -> Result<(), String> {
        let ops = self.ap_ops(ap, reg, Op::write(0, value));
        self.transfer(&ops).map(drop)
    }

    fn set_csw(&mut self, increment: bool) -> Result<(), String> {
        let csw = CSW_WORD | if increment { CSW_INCREMENT } else { 0 };
        if self.csw != Some(csw) {
            self.write_ap(MEM_AP, AP_CSW, csw)?;
            self.csw = Some(csw);
        }
        Ok(())
    }

    pub fn read32(&mut self, address: u32) -> Result<u32, String> {
        self.set_csw(false)?;
        let mut ops = self.ap_ops(MEM_AP, AP_TAR, Op::write(0, address));
        ops.extend(self.ap_ops(MEM_AP, AP_DRW, Op::read(0)));
        Ok(*self.transfer(&ops)?.last().expect("one read"))
    }

    pub fn write32(&mut self, address: u32, value: u32) -> Result<(), String> {
        self.set_csw(false)?;
        let mut ops = self.ap_ops(MEM_AP, AP_TAR, Op::write(0, address));
        ops.extend(self.ap_ops(MEM_AP, AP_DRW, Op::write(0, value)));
        self.transfer(&ops).map(drop)
    }

    /// Writes each `(address, value)` in turn, and after each one waits for
    /// bit 0 of the word at `ready` to read 1, which is how the nRF's NVMC
    /// says it's finished. Packs as many as fit into each packet.
    pub fn write_each_and_wait(&mut self, writes: &[(u32, u32)], ready: u32) -> Result<(), String> {
        self.set_csw(false)?;
        let tar = REQ_AP | AP_TAR;
        let drw = REQ_AP | AP_DRW;
        let per_write = |address, value| {
            [
                Op::write(tar, address),
                Op::write(drw, value),
                Op::write(tar, ready),
                Op::write(drw | REQ_READ | REQ_MATCH, 1),
            ]
        };
        let each: usize = per_write(0, 0).iter().map(Op::len).sum();

        // The probe keeps the match mask, and SELECT, between transfers, so
        // only the first packet carries them. Every AP register used here is
        // in bank 0.
        let mut first: Vec<Op> = self.select(MEM_AP, AP_TAR).into_iter().collect();
        first.push(Op::write(REQ_MATCH_MASK, 1));
        let mut rest = writes;
        while !rest.is_empty() {
            let mut ops = std::mem::take(&mut first);
            let room = self.packet - 3 - ops.iter().map(Op::len).sum::<usize>();
            let (chunk, after) = rest.split_at((room / each).clamp(1, rest.len()));
            for &(address, value) in chunk {
                ops.extend(per_write(address, value));
            }
            self.transfer(&ops)?;
            rest = after;
        }
        Ok(())
    }

    /// Waits, in the probe, for `ap`'s register `reg` to read `value` under
    /// `mask`, retrying as often as TRANSFER_CONFIGURE allows.
    pub fn wait_ap(&mut self, ap: u8, reg: u8, mask: u32, value: u32) -> Result<(), String> {
        let mut ops = vec![Op::write(REQ_MATCH_MASK, mask)];
        ops.extend(self.ap_ops(ap, reg, Op::write(REQ_READ | REQ_MATCH, value)));
        self.transfer(&ops).map(drop)
    }

    /// Reads `count` words from `address` up: a TAR write and as many DRW
    /// reads as the response has room for, in each DAP_Transfer.
    pub fn read_block(&mut self, address: u32, count: usize) -> Result<Vec<u32>, String> {
        self.set_csw(true)?;
        let per_packet = (self.packet - 3) / 4;
        let mut words = Vec::with_capacity(count);
        while words.len() < count {
            let at = address + 4 * u32::try_from(words.len()).expect("fits");
            let to_wrap = (INCREMENT_WRAP - at % INCREMENT_WRAP) / 4;
            let n = (count - words.len()).min(per_packet).min(to_wrap as usize);
            let mut ops = self.ap_ops(MEM_AP, AP_TAR, Op::write(0, at));
            for _ in 0..n {
                ops.extend(self.ap_ops(MEM_AP, AP_DRW, Op::read(0)));
            }
            words.extend(self.transfer(&ops)?);
        }
        Ok(words)
    }
}

fn describe(ack: u8, done: usize, expected: usize) -> String {
    let what = if ack & RESP_MISMATCH != 0 {
        "a register never reached the value it was waiting for"
    } else if ack & RESP_PROTOCOL_ERROR != 0 {
        "an SWD parity or protocol error"
    } else {
        match ack & 0x07 {
            ACK_OK => "fewer transfers than asked for",
            ACK_WAIT => "the target kept answering WAIT",
            ACK_FAULT => "the target answered FAULT",
            _ => "no answer from the target",
        }
    };
    format!("{what} (after {done} of {expected} transfers, response {ack:#04x})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    /// The nRF52's CTRL-AP, and what its IDR reads.
    const CTRL_AP: u8 = 1;
    const IDR: u8 = 0xFC;

    fn connected(fake: Fake) -> Dap<Fake> {
        let mut dap = Dap::new(fake);
        dap.connect(1_000_000).unwrap();
        dap
    }

    #[test]
    fn a_failed_transfer_doesnt_leave_select_pointing_at_the_wrong_ap() {
        let mut dap = connected(Fake::nrf52(false));
        dap.link.wait_next_transfer = true;
        // The SELECT for AP 1 was in this transfer, which never ran.
        assert!(dap.read_ap(CTRL_AP, IDR).is_err());
        assert_eq!(dap.read_ap(CTRL_AP, IDR), Ok(0x0288_0000));
    }
}
