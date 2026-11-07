# Hardware

What's on each micro:bit that firmware here cares about, and how sure I am about each bit. "Confirmed" means a primary source says so: the micro:bit Foundation's [tech site](https://tech.microbit.org/hardware/), Nordic's documentation, or the nrf-rs BSP that `board` is built on. "Likely" means it follows from one of those, or from what I know of the chips, and I haven't found it written down. None of it has been checked on a real board by this repo.

nrf-rs citations are pinned to [`e4265b5`](https://github.com/nrf-rs/microbit/tree/e4265b5c9607ca52269184d14b2ae168958e8c37) (October 2026), which is `microbit` and `microbit-v2` 0.16.0.

## Telling them apart

There are four board revisions in the wild: 1.3x and 1.5 (both "v1"), and 2.0 and 2.2x ("v2"). The v2 has a notched edge connector ([schematic notes](https://tech.microbit.org/hardware/schematic/#edge-connector)), a speaker and microphone, and a red power LED on the back. Confirmed. Firmware here only cares whether it's a v1 or a v2: the revisions within each share a target chip, and differ in sensors or the interface chip.

## The target chip

| | v1 | v2 |
|---|---|---|
| Part | nRF51822-QFAA-R rev 3 | nRF52833 |
| Core | Cortex-M0 | Cortex-M4 with FPU |
| Clock | 16 MHz | 64 MHz |
| Flash | 256 KB | 512 KB |
| RAM | 16 KB | 128 KB |
| Rust target | `thumbv6m-none-eabi` | `thumbv7em-none-eabihf` |

The part, core, clock and sizes are from the [1.5](https://tech.microbit.org/hardware/1-5-revision/) and [2.2x](https://tech.microbit.org/hardware/2-2-revision/) revision pages (the [1.3x](https://tech.microbit.org/hardware/1-3-revision/) and [2.0](https://tech.microbit.org/hardware/2-0-revision/) pages say the same). Confirmed. Flash starts at 0x0 and RAM at 0x20000000 on both, which is where `xtask` counts them from and what the HALs' `memory.x` files say. Likely, from the HALs rather than a Nordic page. The targets are from the [nrf-rs README](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/README.md). Confirmed.

### Access port protection on the v2

The nRF52833 comes in two kinds here. Build codes Axx and earlier (revision 1) have protection off by default and controlled by the UICR alone. Build codes Bxx and later (revision 2) have the "hardened" kind, which is on by default and controlled by hardware and software together ([PS, Debug and trace](https://docs.nordicsemi.com/r/bundle/ps_nrf52833/page/dif.html); Nordic's [improved APPROTECT post](https://devzone.nordicsemi.com/nordic/nordic-blog/b/blog/posts/working-with-the-nrf52-series-improved-approtect) names revision 2 as Bx0). Confirmed. Which one a given micro:bit v2 has isn't written down anywhere I've found: the Foundation's [hardware repo](https://github.com/microbit-foundation/microbit-v2-hardware) BOMs list `nRF52833-QIAA` (2.0) and `nRF52833-QIAA-R` (2.21) with no build code. So treat either as possible on any v2. Likely.

On a hardened chip:

- A CTRL-AP ERASEALL erases flash, UICR and RAM and opens the port until a pin, power or brownout reset, a watchdog reset or a wake from System OFF (PS, same page). Confirmed. A soft reset (SYSRESETREQ, or the CTRL-AP's RESET register) isn't on that list, so it probably leaves the port open. Likely, from the list, and not tried.
- Keeping it open across those resets takes two things: UICR.APPROTECT (0x10001208) set to HwDisabled (0x5A), and firmware writing SwDisable (0x5A) to APPROTECT.DISABLE (0x40000558) as it boots (PS, same page; addresses from nrfx's [`nrf52833.h`](https://github.com/NordicSemiconductor/nrfx/blob/e04a873799c0e7206166d62e2e4f185868acf8f9/bsp/stable/mdk/nrf52/nrf52833/nrf52833.h)). Confirmed. With only the UICR half, the port locks again at the next pin or power reset. Likely, since the PS describes both halves but not that case on its own. `nrfjprog --recover` does both: it writes HwDisabled and also programs a small image to keep it open (the blog post). Confirmed.
- Nordic's startup code only touches APPROTECT.DISABLE on a hardened chip, which it spots by FICR words 0x10000130 reading 0x0D and 0x10000134 reading anything but 0. It then copies UICR.APPROTECT into DISABLE ([`nrf52_erratas.h`](https://github.com/NordicSemiconductor/nrfx/blob/e04a873799c0e7206166d62e2e4f185868acf8f9/bsp/stable/mdk/nrf52/nrf52_erratas.h) `nrf52_configuration_249`, [`system_nrf52_approtect.h`](https://github.com/NordicSemiconductor/nrfx/blob/e04a873799c0e7206166d62e2e4f185868acf8f9/bsp/stable/mdk/nrf52/system_nrf52_approtect.h)). Confirmed as what nrfx does. Those FICR words aren't in the PS. The nRF52833 PAC has no APPROTECT peripheral and nrf-hal 0.19 doesn't write it, so Rust firmware has to do it itself. Confirmed from the crate sources.

`cargo xtask flash` writes the UICR half on a hardened chip and `board::Board::take` does the firmware half, both gated on that FICR check. Neither has run on a real v2.

The M0 has no FPU and no hardware divide, so floats and division are software routines on the v1. That's the Cortex-M0, not something the micro:bit pages say. Likely, and it's why the v1 is the board to watch for size and speed.

## The LED matrix

Both boards have 25 red LEDs in a 5×5 grid. How they're wired differs:

- **v1:** "connected to the micro:bit as a 3x9 matrix" ([1.5 page](https://tech.microbit.org/hardware/1-5-revision/#display)), "9 columns by 3 rows. Row 2 Col 8, and Row 2 Col 9 are not used" ([schematic notes](https://tech.microbit.org/hardware/schematic/#leds)). Rows are P0.13 to P0.15 and columns P0.04 to P0.12 ([nrf-rs `v1/gpio.rs`](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/microbit-common/src/v1/gpio.rs)). The mapping from electrical row and column to the LED you see is scrambled, and nrf-rs keeps it as a table (`MICROBIT_LED_LAYOUT` in [`display/nonblocking/matrix.rs`](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/microbit-common/src/display/nonblocking/matrix.rs)). Confirmed.
- **v2:** "implemented as a 5x5 matrix" ([schematic notes](https://tech.microbit.org/hardware/schematic/#leds)). Rows 1 to 5 are P0.21, P0.22, P0.15, P0.24, P0.19, and columns 1 to 5 are P0.28, P0.11, P0.31, P1.05, P0.30 ([V2 pinmap](https://tech.microbit.org/hardware/schematic/#v2-pinmap), and nrf-rs [`v2/gpio.rs`](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/microbit-common/src/v2/gpio.rs) agrees). Confirmed.

On both, the matrix is multiplexed in software: only one row is lit at a time, so something has to keep refreshing it. Some column pins are also edge connector pins, so using those pins means turning the display off ([schematic notes](https://tech.microbit.org/hardware/schematic/#leds)). Confirmed. `board` leaves both of these to the BSP's display driver.

The v2 page lists brightness as "software controlled up to 255 steps" and the v1 as "10 steps". Those are what the micro:bit's own runtime does, not limits of the hardware. Likely. `board` gets ten levels by keeping each LED on for part of its row's 1 ms, from 8 µs to the whole of it, scanning at 333 Hz on the v1 and 200 Hz on the v2. On a v1.5 that shows no flicker and no ghosting that anyone could see. Confirmed on a v1.5, not yet tried on a v2.

## Buttons

A and B are on the front, with "external 4K7" pull-ups, reading 1 released and 0 pressed ([2.2x page](https://tech.microbit.org/hardware/2-2-revision/#buttons); the 1.5 page says the same). Confirmed. That's why the BSPs configure them as floating inputs.

| | v1 | v2 |
|---|---|---|
| A | P0.17 | P0.14 |
| B | P0.26 | P0.23 |

v1 from nrf-rs `v1/gpio.rs`, v2 from the [V2 pinmap](https://tech.microbit.org/hardware/schematic/#v2-pinmap) and `v2/gpio.rs`. Confirmed. They're also edge connector pins 5 and 11 on both.

The button on the back is reset. It's "connected to the interface processor and to the nRF52 processor for system reset purposes" ([2.2x page](https://tech.microbit.org/hardware/2-2-revision/#buttons)). Confirmed. On a v2, holding it fades the power LED and, once that goes dark, the board's "in power-saving sleep mode" ([microbit.org user guide](https://microbit.org/get-started/user-guide/features-in-depth/)). Confirmed as what the guide says. The interface chip does that, not the firmware. Likely.

## v2 only

From the [V2 pinmap](https://tech.microbit.org/hardware/schematic/#v2-pinmap) and the [2.2x page](https://tech.microbit.org/hardware/2-2-revision/), confirmed unless I say otherwise:

| Pin | Name | What |
|---|---|---|
| P0.00 | SPEAKER | Magnetic speaker (MLT-8530, resonant at 2700 Hz), driven by PWM |
| P0.05 | MIC_IN | MEMS microphone (Knowles SPU0410LR5H), AC-coupled, analogue |
| P0.20 | RUN_MIC | Powers the microphone. The microphone LED on the front shows when it's powered |
| P1.04 | FACE_TOUCH | The gold logo as a touch input, with a 10 MΩ pull-up |
| P0.25 | COMBINED_SENSOR_INT | Open drain, active low, shared by the motion sensor and the interface chip |

The red power LED on the back "shows when your micro:bit has power" ([user guide](https://microbit.org/get-started/user-guide/features-in-depth/)). It isn't on the target's pinmap, so firmware can't drive it. Likely.

P0.09 and P0.10 can be the nRF52833's NFC antenna pins, and the schematic notes say NFC is "disabled by default". On the nRF52 that's a UICR setting (NFCPINS) in the flashed image, and nrf-hal has a feature for it. I haven't checked what the BSP or a blank chip does, so treat those two pins (edge connector 9 and 8) as unknown until someone does. Likely.

## Motion sensor and I2C

- v1.3x: an NXP MMA8653FC accelerometer and a separate MAG3110 magnetometer ([1.3x page](https://tech.microbit.org/hardware/1-3-revision/)). Confirmed.
- v1.5: an ST LSM303AGR, accelerometer and magnetometer in one chip, as U2 ([V1.5 BOM](https://github.com/bbcmicrobit/hardware/blob/426f98b6d0e9c83367d7c4094d0b4a33b3665525/V1.5/Bill%20of%20Materials-BBC-microbit_v1.5.csv) line 37, [V1.5 schematic](https://github.com/bbcmicrobit/hardware/blob/426f98b6d0e9c83367d7c4094d0b4a33b3665525/V1.5/SCH_BBC-Microbit_V1.5.PDF) sheet 3). Confirmed. The [1.5 page](https://tech.microbit.org/hardware/1-5-revision/)'s table says "LSM303GR", a typo: its text says LSM303AGR, and that "all V1.5 boards have been manufactured with the LSM303AGR" to date. The schematic also has an unfitted FXOS8700CQ footprint (U1), which isn't in the BOM.
- v2: a footprint for either an ST LSM303AGR or an NXP FXOS8700CQ, and "only one sensor will ever be placed" ([2.2x page](https://tech.microbit.org/hardware/2-2-revision/#motion-sensor)). Confirmed. So a v2 firmware that reads the sensor has to detect which one it's got, as the micro:bit's own runtime does.

On the v1 the sensor's I2C bus is also the edge connector's (SCL P0.00, SDA P0.30, from nrf-rs). On the v2 they're separate: internal SCL P0.08 and SDA P0.16, external SCL P0.26 and SDA P1.00 ([V2 pinmap](https://tech.microbit.org/hardware/schematic/#v2-pinmap)). The internal bus also reaches the interface chip, with "a custom protocol" ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). Confirmed.

On the v1.5 the bus has 4k7 pull-ups, and the accelerometer's INT1 goes to P0.28 (a net still named `MMA8653FC_INT1` from the 1.3) and the magnetometer's interrupt to P0.29. INT2 isn't wired ([V1.5 schematic](https://github.com/bbcmicrobit/hardware/blob/426f98b6d0e9c83367d7c4094d0b4a33b3665525/V1.5/SCH_BBC-Microbit_V1.5.PDF) sheets 3 and 5). Confirmed. `board` polls, so it uses neither.

### The LSM303AGR

From the [ST datasheet](https://www.st.com/resource/en/datasheet/lsm303agr.pdf) (DocID027765 Rev 11). Confirmed for the chip.

- The accelerometer is at 7-bit address 0x19 and the magnetometer at 0x1E (section 6.1). The V1.5 schematic annotates the same.
- `WHO_AM_I_A` (0x0F) reads 0x33 (section 8.4).
- `CTRL_REG1_A` (0x20) sets the data rate in its top four bits (0101 is 100 Hz) and turns on X, Y and Z in its bottom three. `CTRL_REG4_A` (0x23) has block data update in bit 7, the range in bits 5:4 (00 is ±2 g) and high resolution in bit 3 (sections 8.6 and 8.9). `board` writes 0x57 and 0x88.
- The outputs are `OUT_X_L_A` to `OUT_Z_H_A` (0x28 to 0x2D), low byte first, two's complement and left-justified. Setting bit 7 of the register address reads on through the next ones (sections 6.1, 8.14 to 8.16).
- High resolution is 12 bits, at 0.98 mg a step at ±2 g, and takes 7 readings' time to settle after it's turned on (Tables 3 and 14).

The micro:bit's runtime turns the chip's axes into the board's in two steps. Its driver first makes (-y, -x, z) of them ([microbit-dal `LSM303Accelerometer.cpp`](https://github.com/lancaster-university/microbit-dal/blob/318c6abbb3d952d3f49a595a726486230e82fb5b/source/drivers/LSM303Accelerometer.cpp), and the same in [codal-core's](https://github.com/lancaster-university/codal-core/blob/master/source/drivers/LSM303Accelerometer.cpp) for the v2). Then a `CoordinateSpace(SIMPLE_CARTESIAN, true, COORDINATE_SPACE_ROTATED_0)` flips and swaps them again ([`CoordinateSystem.cpp`](https://github.com/lancaster-university/codal-core/blob/master/source/types/CoordinateSystem.cpp)). Confirmed for the source. Put together, what MakeCode reports is the chip's (x, -y, z), and that's what `Tilt` does. On a v1.5 that puts x growing as the right edge (button B's) tips down and y growing as the edge connector tips down, which is what MakeCode's gestures expect ([`Accelerometer.cpp`](https://github.com/lancaster-university/codal-core/blob/master/source/driver-models/Accelerometer.cpp)). Confirmed on a v1.5 with the `level` firmware. MakeCode's docs say z reads -1023 lying face up ([acceleration](https://makecode.microbit.org/reference/input/acceleration)). Likely for `Tilt`, as nothing here has shown z.

A reset restarts the nRF but not the sensor, so a reset in the middle of a read can leave the LSM303AGR holding SDA low. After that the nRF51's TWI waits forever for the bus, and a v1.5 hung like that until `board` started clocking SCL nine times and sending a STOP before turning the TWI on. Confirmed on a v1.5. That first hang started with nrf-hal 0.19's embedded-hal `I2c` for the nRF51 TWI, which reads without the shortcuts the nRF51 needs to pause the bus between bytes. `board` calls the HAL's own `write_then_read`, which uses them. Likely: the fix and the recovery went on together, so the board never showed which one mattered.

Started cold, from a battery or a fresh USB plug, a v1.5 didn't answer the first probe of `WHO_AM_I_A`, while a reset with power already on always worked. Trying again every 5 ms, for up to 50 ms, fixed it. Confirmed on a v1.5. The cause is likely the sensor still booting when the nRF reaches it, about a millisecond after power-on, but nothing here has measured how long it takes.

The runtime finds its sensor by probing each chip's `WHO_AM_I` in turn: MMA8653 (7-bit 0x1D, register 0x0D, 0x5A), then LSM303 (0x19, 0x0F, 0x33), then FXOS8700 (0x1E, 0x0D, 0xC7) ([microbit-dal `MicroBitAccelerometer.cpp`](https://github.com/lancaster-university/microbit-dal/blob/318c6abbb3d952d3f49a595a726486230e82fb5b/source/drivers/MicroBitAccelerometer.cpp)). Confirmed for the runtime. `board` only probes for the LSM303AGR.

## Serial

The interface chip passes the target's UART through to USB. Only TX and RX are wired, "so there is no hardware flow control" and "at higher serial speeds (baud rates above 9600), the micro:bit may drop characters being sent to it" ([DAPLink page](https://tech.microbit.org/software/daplink-interface/#uart-connection)). Confirmed.

| | v1 | v2 |
|---|---|---|
| Target TX | P0.24 | P0.06 |
| Target RX | P0.25 | P1.08 |

v1 from nrf-rs, v2 from nrf-rs and the pinmap. Confirmed. The pinmap names these from the interface's side, so its `UART_INT_RX` is the target's TX.

## The interface chip

The USB port doesn't go to the target. It goes to a second microcontroller running Arm's [DAPLink](https://tech.microbit.org/software/daplink-interface/), which gives you the MICROBIT drive for drag-and-drop flashing, the serial port, and a CMSIS-DAP debug probe on the target's SWD pins. Only the debug probe is used here. It has no connection to the edge connector. Confirmed.

| Revision | Interface chip | Shipped DAPLink | Latest tested |
|---|---|---|---|
| 1.3, 1.3b | NXP MKL26Z128VFM4 (Cortex-M0+) | 0234, 0241 | 0253 |
| 1.5 | NXP MKL26Z128VFM4 | 0249 | 0253 |
| 2.00 | NXP MKL27Z256VFM4 (Cortex-M0+) | 0255 | 0255 |
| 2.20 | Nordic nRF52820-QDAA, or nRF52833-QDAA on a limited run | 0256 | 0257 |
| 2.21 | the same | 0257 | 0257 |

Chips from the revision pages, versions from the [DAPLink page](https://tech.microbit.org/software/daplink-interface/) (which also lists a 0258 beta for 2.00 and 2.2x). Confirmed. The [firmware download page](https://microbit.org/get-started/user-guide/firmware/) offers 0249 for every v1, not 0253, so "latest tested" isn't what you can download. Confirmed. A v1.5 here runs 0249 on bootloader 0243. In MAINTENANCE mode its `DETAILS.TXT` says `Daplink Mode: Bootloader` and `USB Interfaces: MSD`, so there's no debug probe until the interface firmware is running. Confirmed on that board. The two 2.2x interface chips run "the same DAPLink firmware image" ([2.2x page](https://tech.microbit.org/hardware/2-2-revision/#interface)). Confirmed.

On the v1, the KL26 also contains the regulator that powers the rest of the board from USB, rated 120 mA. The v2 has its own regulator, rated 300 mA ([schematic notes](https://tech.microbit.org/hardware/schematic/#power-supply)). Confirmed.

The 1.5 and 2.0 pages list "JLink/OB (via different firmware)" as a debug option. That means replacing DAPLink, which nothing here needs. Confirmed as possible, and not something I've tried.

So between the v2 revisions the target chip is the same and the interface chip isn't. Nothing I've found says that changes anything for firmware, apart from which interface firmware file to download.

The MICROBIT drive's `DETAILS.TXT` has a "Unique ID" line that starts with a four-digit board ID. The same ID is the probe's USB serial number, which is how `cargo xtask flash` tells the boards apart. The Foundation's [`universal-hex.ts`](https://github.com/microbit-foundation/microbit-universal-hex/blob/main/src/universal-hex.ts) says "For micro:bit V1 (v1.3, v1.3B and v1.5) the `boardId` is `0x9900`, and for V2 `0x9903`", and also lists 0x9901 as a V1 ID. Confirmed for those three. A v1.5 here (bootloader 0243, interface 0253) reads `Unique ID: 9901000052374e45…` and `HIC ID: 97969901`, so the line's format and the 1.5's ID are confirmed on a real board. I haven't found the IDs for each v2 revision, so `xtask` also takes 9904 to 9906 as a v2. Likely.

## Temperature

Both read their core temperature from the Nordic chip: -25 to 75 °C, ±4 °C uncalibrated on the v1, and -40 to 105 °C, ±5 °C on the v2, in 0.25 °C steps ([1.5](https://tech.microbit.org/hardware/1-5-revision/), [2.2x](https://tech.microbit.org/hardware/2-2-revision/) pages). Confirmed.

## Open questions

- What does the BSP do with P0.09 and P0.10 on the v2, and does a firmware need to set NFCPINS in the UICR to use them as GPIO?
- Does the v2's LSM303AGR read the same way round as the v1.5's? The runtime treats them the same, but `level` has only run on a v1.5.
- Do the v2.0 and v2.2x put the interface chip at the same I2C address on the internal bus? Nothing here talks to it yet.
- What board IDs do the 2.0, 2.20 and 2.21 report in `DETAILS.TXT`? Plugging each one in would settle it.
- Which v2 revisions have a hardened nRF52833 (build code Bxx)? Reading FICR 0x10000134 over SWD on each would settle it, and so would the build code printed on the chip.
