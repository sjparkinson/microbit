//! Intel HEX, the format the micro:bit's MICROBIT drive takes: drag a .hex on
//! and the interface chip programs it.

use std::fmt::Write;

use crate::elf::Segment;

const DATA: u8 = 0x00;
const END: u8 = 0x01;
const EXTENDED_LINEAR_ADDRESS: u8 = 0x04;

/// Encodes `segments` at their addresses, 16 bytes a line. Gaps between
/// segments stay gaps, so they're left as they are rather than written with
/// 0xFF. A line never crosses a 64 KB boundary, and an extended linear address
/// record comes first whenever the upper 16 bits change (the nRF52833's flash
/// is 512 KB, and UICR sits at 0x10001000).
pub fn encode(segments: &[Segment]) -> String {
    let mut out = String::new();
    let mut upper = None;
    for segment in segments {
        let mut address = segment.address;
        let mut rest = segment.data.as_slice();
        while !rest.is_empty() {
            let [hi, lo, ..] = address.to_be_bytes();
            if upper != Some([hi, lo]) {
                record(&mut out, 0, EXTENDED_LINEAR_ADDRESS, &[hi, lo]);
                upper = Some([hi, lo]);
            }
            let to_boundary = 0x1_0000 - (address & 0xFFFF);
            let len = rest.len().min(16).min(to_boundary as usize);
            let (line, after) = rest.split_at(len);
            record(&mut out, address as u16, DATA, line);
            address = address.wrapping_add(len as u32);
            rest = after;
        }
    }
    record(&mut out, 0, END, &[]);
    out
}

fn record(out: &mut String, address: u16, kind: u8, data: &[u8]) {
    let [hi, lo] = address.to_be_bytes();
    let header = [data.len() as u8, hi, lo, kind];
    let checksum = sum(&header).wrapping_add(sum(data)).wrapping_neg();

    out.push(':');
    for byte in header.iter().chain(data) {
        let _ = write!(out, "{byte:02X}");
    }
    let _ = writeln!(out, "{checksum:02X}");
}

/// Byte sum, mod 256. A valid record, checksum included, sums to zero.
fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |sum, b| sum.wrapping_add(*b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(address: u32, data: &[u8]) -> Segment {
        Segment {
            address,
            data: data.to_vec(),
        }
    }

    #[test]
    fn empty_is_the_end_record() {
        assert_eq!(encode(&[]), ":00000001FF\n");
    }

    #[test]
    fn splits_into_sixteen_byte_lines_with_checksums() {
        // The start of a Cortex-M vector table: initial SP 0x20020000, then
        // the reset vector 0x00000101.
        let mut data = vec![0x00, 0x00, 0x02, 0x20, 0x01, 0x01, 0x00, 0x00];
        data.extend([0xFF; 10]);
        assert_eq!(
            encode(&[segment(0, &data)]),
            ":020000040000FA\n\
             :100000000000022001010000FFFFFFFFFFFFFFFFD4\n\
             :02001000FFFFF0\n\
             :00000001FF\n"
        );
    }

    #[test]
    fn every_record_sums_to_zero() {
        let data: Vec<u8> = (0..=255).collect();
        for line in encode(&[segment(0x7_FF80, &data)]).lines() {
            let bytes: Vec<u8> = (1..line.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&line[i..i + 2], 16).unwrap())
                .collect();
            assert_eq!(sum(&bytes), 0, "{line}");
            assert_eq!(usize::from(bytes[0]) + 5, bytes.len(), "{line}");
        }
    }

    #[test]
    fn breaks_lines_at_64_kb_and_says_where_next() {
        let hex = encode(&[segment(0xFFF8, &[0xAB; 16])]);
        let lines: Vec<&str> = hex.lines().collect();
        assert_eq!(lines.len(), 5, "{hex}");
        assert_eq!(lines[0], ":020000040000FA");
        assert!(lines[1].starts_with(":08FFF800"), "{}", lines[1]);
        assert_eq!(lines[2], ":020000040001F9");
        assert!(lines[3].starts_with(":08000000"), "{}", lines[3]);
        assert_eq!(lines[4], ":00000001FF");
    }

    #[test]
    fn leaves_gaps_and_only_repeats_the_upper_address_when_it_changes() {
        let hex = encode(&[
            segment(0x0000, &[1]),
            segment(0x0100, &[2]),
            segment(0x1000_1014, &[3]),
        ]);
        assert_eq!(
            hex,
            ":020000040000FA\n\
             :0100000001FE\n\
             :0101000002FC\n\
             :020000041000EA\n\
             :0110140003D8\n\
             :00000001FF\n"
        );
    }
}
