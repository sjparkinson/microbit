# Flashing

There's nothing to install and no programmer to buy. The micro:bit's USB port goes to an interface chip running DAPLink, which is a CMSIS-DAP debug probe wired to the target's SWD pins ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). A data-capable USB cable is all you need. Some cheap micro-USB cables only carry power, and with one of those nothing shows up at all.

Flashing can't hurt the board. The interface chip is separate from the target, so "no matter what code you run on your micro:bit, or how you manage to crash the device, you can always still put a new program on using the USB connection" (same page).

## cargo xtask flash

```sh
cargo xtask flash heartbeat             # v2
cargo xtask flash heartbeat --board v1  # v1
```

The CMSIS-DAP client lives in `xtask` itself (`xtask/src/hid.rs`, `dap.rs` and `nrf.rs`). In order, it:

1. finds the micro:bit on USB (ID `0d28:0204`) and reads the board ID from the first four digits of its serial number, stopping if it's the other board;
2. builds the release image;
3. connects over SWD at 1 MHz and powers up the chip's debug port;
4. halts the core, resets it with a vector catch so it stops before running anything, and checks its CPUID says Cortex-M0 (v1) or Cortex-M4 (v2);
5. reads the flash page size and count from the FICR, erases each page the image touches through the NVMC, and writes the image a word at a time, waiting for the NVMC after each;
6. reads every word back and compares;
7. lets go of the core and resets it, so the firmware starts.

Only flash gets programmed. An image that puts anything outside it, such as UICR settings, fails at `cargo xtask build`, so the .hex it writes never holds something the debug probe would skip. Nothing here sets UICR yet; when something does, the flasher will need to learn it.

Pages the image doesn't touch are left alone. On the v2 it checks the nRF52's access port protection first, and if it's on it erases the whole chip through the CTRL-AP, which is the only way back in. On an nRF52833 with the hardened protection (revision 2, see [hardware.md](hardware.md#access-port-protection-on-the-v2)) it then sets UICR.APPROTECT to HwDisabled, and `board` writes APPROTECT.DISABLE as the firmware boots, so the port stays open after you unplug it and the next flash doesn't need to erase everything again. A firmware that never calls `board::Board::take` leaves the port to lock at the next power-up, and the flash after that erases the whole chip again. All of this has only run against the fake chip in `xtask/src/fake.rs`, since only a v1 has been tried so far.

The board IDs come from the micro:bit Foundation's [universal hex code](https://github.com/microbit-foundation/microbit-universal-hex/blob/main/src/universal-hex.ts), which gives 9900 and 9901 for the v1 and 9903 for the v2. A v1.5 here reports 9901. If `flash` doesn't recognise the ID it warns and goes by the CPUID alone.

On Linux it uses `/dev/hidrawN`, which needs a udev rule for `0d28:0204` to work without root. That path hasn't been tried yet.

## Updating the interface firmware

Worth doing once on any board you didn't buy new. The current versions are 0253 for every v1, 0255 for a 2.00 and 0257 for a 2.2x ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). `DETAILS.TXT` on the MICROBIT drive says which one you've got.

1. Download the right file for your board from [microbit.org's firmware page](https://microbit.org/get-started/user-guide/firmware/). The page tells you how to identify the revision.
2. Unplug the micro:bit. Hold the reset button on the back, and plug it in while still holding it. A drive called MAINTENANCE appears instead of MICROBIT.
3. Drag the firmware hex onto MAINTENANCE and wait for the yellow LED to stop flashing. The board resets and comes back as MICROBIT.

Only use firmware from microbit.org. The DAPLink page warns that firmware from anywhere else "could damage your micro:bit, or make it impossible to re-flash", and that's the one way to break the USB side.

## When it doesn't work

- **"there's no micro:bit on USB".** Check the cable carries data: does the MICROBIT drive appear? On Linux, check the udev rule.
- **"the target answered FAULT" or "no answer from the target".** Unplug it, plug it back in and try again. If a v2 keeps doing it, its access port protection may be on in a way the CTRL-AP path doesn't handle.
- **It says the micro:bit's in MAINTENANCE mode.** The reset button was held while it powered up. Unplug and plug back in without touching it.

## Sandbox note

Claude Code's sandbox blocks USB HID, so `cargo xtask flash` needs the sandbox off. If it fails there, run the command yourself.
