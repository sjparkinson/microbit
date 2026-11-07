# Hardware

What's on each micro:bit that firmware here cares about, and how sure I am about each bit. "Confirmed" means a primary source says so: the micro:bit Foundation's [tech site](https://tech.microbit.org/hardware/), Nordic's documentation, probe-rs's target files, or the nrf-rs BSP that `board` is built on. "Likely" means it follows from one of those, or from what I know of the chips, and I haven't found it written down. None of it has been checked on a real board by this repo.

nrf-rs citations are pinned to [`e4265b5`](https://github.com/nrf-rs/microbit/tree/e4265b5c9607ca52269184d14b2ae168958e8c37) (October 2026), which is `microbit` and `microbit-v2` 0.16.0. probe-rs citations are to its `master` branch as of November 2026.

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
| probe-rs chip | `nRF51822_xxAA` | `nRF52833_xxAA` |

The part, core, clock and sizes are from the [1.5](https://tech.microbit.org/hardware/1-5-revision/) and [2.2x](https://tech.microbit.org/hardware/2-2-revision/) revision pages (the [1.3x](https://tech.microbit.org/hardware/1-3-revision/) and [2.0](https://tech.microbit.org/hardware/2-0-revision/) pages say the same). Confirmed. probe-rs's [`nRF51_Series.yaml`](https://github.com/probe-rs/probe-rs/blob/master/probe-rs/targets/nRF51_Series.yaml) gives `nRF51822_xxAA` flash at 0x0 to 0x40000 and RAM at 0x20000000 to 0x20004000, and [`nRF52_Series.yaml`](https://github.com/probe-rs/probe-rs/blob/master/probe-rs/targets/nRF52_Series.yaml) gives `nRF52833_xxAA` 0x0 to 0x80000 and 0x20000000 to 0x20020000, which agree. Confirmed. The targets are from the [nrf-rs README](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/README.md). Confirmed.

The M0 has no FPU and no hardware divide, so floats and division are software routines on the v1. That's the Cortex-M0, not something the micro:bit pages say. Likely, and it's why the v1 is the board to watch for size and speed.

## The LED matrix

Both boards have 25 red LEDs in a 5×5 grid. How they're wired differs:

- **v1:** "connected to the micro:bit as a 3x9 matrix" ([1.5 page](https://tech.microbit.org/hardware/1-5-revision/#display)), "9 columns by 3 rows. Row 2 Col 8, and Row 2 Col 9 are not used" ([schematic notes](https://tech.microbit.org/hardware/schematic/#leds)). Rows are P0.13 to P0.15 and columns P0.04 to P0.12 ([nrf-rs `v1/gpio.rs`](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/microbit-common/src/v1/gpio.rs)). The mapping from electrical row and column to the LED you see is scrambled, and nrf-rs keeps it as a table (`MICROBIT_LED_LAYOUT` in [`display/nonblocking/matrix.rs`](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/microbit-common/src/display/nonblocking/matrix.rs)). Confirmed.
- **v2:** "implemented as a 5x5 matrix" ([schematic notes](https://tech.microbit.org/hardware/schematic/#leds)). Rows 1 to 5 are P0.21, P0.22, P0.15, P0.24, P0.19, and columns 1 to 5 are P0.28, P0.11, P0.31, P1.05, P0.30 ([V2 pinmap](https://tech.microbit.org/hardware/schematic/#v2-pinmap), and nrf-rs [`v2/gpio.rs`](https://github.com/nrf-rs/microbit/blob/e4265b5c9607ca52269184d14b2ae168958e8c37/microbit-common/src/v2/gpio.rs) agrees). Confirmed.

On both, the matrix is multiplexed in software: only one row is lit at a time, so something has to keep refreshing it. Some column pins are also edge connector pins, so using those pins means turning the display off ([schematic notes](https://tech.microbit.org/hardware/schematic/#leds)). Confirmed. `board` leaves both of these to the BSP's display driver.

The v2 page lists brightness as "software controlled up to 255 steps" and the v1 as "10 steps". Those are what the micro:bit's own runtime does, not limits of the hardware. Likely.

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
- v1.5: one combined chip. The page says "LSM303GR", which I read as a typo for the LSM303AGR the v2 uses. Likely.
- v2: a footprint for either an ST LSM303AGR or an NXP FXOS8700CQ, and "only one sensor will ever be placed" ([2.2x page](https://tech.microbit.org/hardware/2-2-revision/#motion-sensor)). Confirmed. So a v2 firmware that reads the sensor has to detect which one it's got, as the micro:bit's own runtime does.

On the v1 the sensor's I2C bus is also the edge connector's (SCL P0.00, SDA P0.30, from nrf-rs). On the v2 they're separate: internal SCL P0.08 and SDA P0.16, external SCL P0.26 and SDA P1.00 ([V2 pinmap](https://tech.microbit.org/hardware/schematic/#v2-pinmap)). The internal bus also reaches the interface chip, with "a custom protocol" ([DAPLink page](https://tech.microbit.org/software/daplink-interface/)). Confirmed.

## Serial

The interface chip passes the target's UART through to USB. Only TX and RX are wired, "so there is no hardware flow control" and "at higher serial speeds (baud rates above 9600), the micro:bit may drop characters being sent to it" ([DAPLink page](https://tech.microbit.org/software/daplink-interface/#uart-connection)). Confirmed.

| | v1 | v2 |
|---|---|---|
| Target TX | P0.24 | P0.06 |
| Target RX | P0.25 | P1.08 |

v1 from nrf-rs, v2 from nrf-rs and the pinmap. Confirmed. The pinmap names these from the interface's side, so its `UART_INT_RX` is the target's TX.

## The interface chip

The USB port doesn't go to the target. It goes to a second microcontroller running Arm's [DAPLink](https://tech.microbit.org/software/daplink-interface/), which gives you the MICROBIT drive for drag-and-drop flashing, the serial port, and a CMSIS-DAP debug probe on the target's SWD pins. That last one is what probe-rs talks to. It has no connection to the edge connector. Confirmed.

| Revision | Interface chip | Shipped DAPLink | Latest tested |
|---|---|---|---|
| 1.3, 1.3b | NXP MKL26Z128VFM4 (Cortex-M0+) | 0234, 0241 | 0253 |
| 1.5 | NXP MKL26Z128VFM4 | 0249 | 0253 |
| 2.00 | NXP MKL27Z256VFM4 (Cortex-M0+) | 0255 | 0255 |
| 2.20 | Nordic nRF52820-QDAA, or nRF52833-QDAA on a limited run | 0256 | 0257 |
| 2.21 | the same | 0257 | 0257 |

Chips from the revision pages, versions from the [DAPLink page](https://tech.microbit.org/software/daplink-interface/) (which also lists a 0258 beta for 2.00 and 2.2x). Confirmed. The two 2.2x interface chips run "the same DAPLink firmware image" ([2.2x page](https://tech.microbit.org/hardware/2-2-revision/#interface)). Confirmed.

On the v1, the KL26 also contains the regulator that powers the rest of the board from USB, rated 120 mA. The v2 has its own regulator, rated 300 mA ([schematic notes](https://tech.microbit.org/hardware/schematic/#power-supply)). Confirmed.

The 1.5 and 2.0 pages list "JLink/OB (via different firmware)" as a debug option. That means replacing DAPLink, which nothing here needs. Confirmed as possible, and not something I've tried.

So between the v2 revisions the target chip is the same and the interface chip isn't. Nothing I've found says that changes anything for firmware, apart from which interface firmware file to download. The 2.20 was the subject of a probe-rs issue about RTT connections dropping after a while ([#1235](https://github.com/probe-rs/probe-rs/issues/1235), open since 2022, with no fix confirmed). Likely to matter only if we start streaming over RTT.

## Temperature

Both read their core temperature from the Nordic chip: -25 to 75 °C, ±4 °C uncalibrated on the v1, and -40 to 105 °C, ±5 °C on the v2, in 0.25 °C steps ([1.5](https://tech.microbit.org/hardware/1-5-revision/), [2.2x](https://tech.microbit.org/hardware/2-2-revision/) pages). Confirmed.

## Open questions

- What does the BSP do with P0.09 and P0.10 on the v2, and does a firmware need to set NFCPINS in the UICR to use them as GPIO?
- Does a 1.5 have an LSM303AGR, as I'm assuming from the "LSM303GR" typo?
- Do the v2.0 and v2.2x put the interface chip at the same I2C address on the internal bus? Nothing here talks to it yet.
- Does probe-rs work against the v1's KL26 DAPLink as well as the v2's? I found no reports either way.
