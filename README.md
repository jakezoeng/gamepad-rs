# gamepad-rs

A Rust + Embassy reimplementation of the **MINIGPA103 USBHID evaluation board** firmware (originally shipped as C + STM32 HAL). Targeting the APM32F103C8 MCU (STM32F103-compatible, Cortex-M3).

> **Reference:** Original firmware from [GZLDLLJ/MiniGamepad_ARM](https://github.com/GZLDLLJ/MiniGamepad_ARM).  
> The MINIGPA103 is a USBHID eval board by 广州联盾电子科技 (LDSCITECHE), featuring an analog joystick, 8 buttons, a WS2812B RGB LED, and a USB full-speed device port.

## Hardware

- **MCU:** APM32F103C8 (STM32F103C8 register-compatible, Cortex-M3 @ 72 MHz, 64K Flash / 20K RAM)
- **Schematic:** `Doc/原理图/SCH_MINIGPV103_Beta_2025-03-22.pdf` (in the original archive)
- **Pin mapping** (verified against `Core/Src/{adc,gpio,tim}.c`):

| Function | Pin | Peripheral |
|---|---|---|
| Joystick X | PA1 | ADC1_IN1 |
| Joystick Y | PA2 | ADC1_IN2 |
| Buttons UP/DN/LF/RG | PB9 / PB8 / PB7 / PB6 | GPIO pull-up, active low |
| Buttons BK/MD/ST/TB | PB5 / PB4 / PB3 / PA15 | GPIO pull-up, active low |
| Button SW1 | PA0 | GPIO pull-down (unused in Eg8) |
| WS2812B DIN | PA6 | TIM3_CH1 (PWM 800 kHz + DMA1_CH6) |
| USB D+/D- | PA12 / PA11 | USB full-speed device |

## Toolchain

```bash
rustup target add thumbv7m-none-eabi
cargo install probe-rs   # flashing / debugging
```

## Build

```bash
cd gamepad-rs
cargo build --release
```

## Flash & Run

```bash
cargo run --release          # flashes via probe-rs and opens defmt RTT log
```

Debugger: ST-Link V2 over SWD. Adjust `--chip` in `.cargo/config.toml` if your probe identifies the chip differently.

## Module Mapping

| Original file | This project |
|---|---|
| `Core/Src/main.c` | `src/main.rs` |
| `Customer/gamepad.c` | `src/gamepad.rs` |
| `Customer/ws281x.c` | `src/ws2812.rs` |
| `USB_DEVICE/App/usbd_custom_hid_if.c` | `src/usb.rs` |
| `Customer/MultiTimer.c` | `embassy_time::Timer` (await directly) |
| `Core/Src/{adc,gpio,tim}.c` | `embassy_stm32` init in `main.rs` |

## Status / TODO

**Done**
- [x] Project skeleton, build, and toolchain setup
- [x] USB HID class + descriptor copied byte-for-byte from original
- [x] Gamepad logic (ADC averaging + hat table + report-on-change)
- [x] Size confirmed: Flash 28.7 KB / 64 KB (45%), RAM 4.7 KB / 20 KB (24%)

**Pending**
- [ ] **First hardware flash test** (ST-Link SWD) and verify Windows gamepad recognition
- [ ] **WS2812B DMA:** `ws2812.rs` is a blocking placeholder with non-compliant timing; needs DMA to stream 24 CCR values
- [ ] **JTAG pins** (PA15/PB3/PB4): confirm embassy-stm32 disables JTAG
- [ ] **Eg10_Xinput** vendor-specific class (separate task)

**Improvements (inspired by [GP2040-CE](https://github.com/OpenStickCommunity/GP2040-CE))**

GP2040-CE targets the RP2040 (264 KB RAM / PIO / dual-role USB); this board's F103-class MCU can't host its full feature set (web configurator, all-console protocols, add-ons). The transferable ideas below are scoped to the hardware:

- [ ] **SOCD cleaning** (Up-priority / Neutral / Second-input) + correct hat mapping — replaces the buggy `hat_lookup`
- [ ] **Button debounce** and 1000 Hz polling (`poll_ms: 1`, matching GP2040-CE latency)
- [ ] **Turbo / button remap / D-pad↔analog toggle**
- [ ] **ADC continuous DMA** instead of serial 10-sample averaging
- [ ] **Config persistence** (flash page) for mode / SOCD / turbo settings
- [ ] **Unit tests** for `map` / `hat_lookup` / SOCD (logic runs on host)

**CRA Compliance (Cyber Resilience Act, EU — enforceable from 2027-12)**

Scope: a USB HID gamepad is a default-class PDE (self-assessment, Module A); not Annex III/IV. The APM32F103C8 lacks hardware Secure Boot, OTP key storage, crypto accelerator, and Armv8-M TrustZone, so TF-A / TF-M cannot run (see `Doc` for the architecture analysis). The following software-level measures are feasible on F103 and sufficient for the default-class self-declaration:

- [ ] **Signed bootloader** — 8–16 KB Ed25519 verifier in Flash @ `0x08000000`; verifies App signature before jump. Public key embedded in bootloader; bootloader is then locked with RDP2 (permanent, non-downgradeable). (#12)
- [ ] **USB DFU upgrade path** — expose a DFU class on `embassy-usb` so App firmware can be updated in the field over USB (CRA: "free security updates" for ≥5 years / product lifetime). Blocked by #12. (#13)
- [ ] **RDP Level 2 lockdown** — production units set RDP2 to block Flash read/erase via SWD and disable debug access. **Warning:** irreversible; must be the last step before shipping. (#14)
- [ ] **SBOM generation** — `cargo install cargo-cyclonedx` → `cargo cyclonedx -f json` in CI; required input for CRA technical documentation. (#15)
- [ ] **SWD disable on production** — disable SWD after provisioning (covered by RDP2, but verify no leftover debug surface). (#16)

Out of scope on this hardware (would require Cortex-M33 + TrustZone-M, e.g. STM32L5/U5, APM32E5/L5): TF-M, hardware-backed Secure Boot, Secure Element, side-channel-resistant key store. Re-evaluate only if the product is reclassified into CRA Annex III high-risk (authentication device, child-safety, CII).

## Known "Original Bug" (intentionally preserved)

`gamepad.rs::hat_lookup` is a 1:1 copy of the truth table in the original `gamepad.c:170-208`. The mapping between button combinations and hat directions is inconsistent in the original code. It is preserved for behavior parity only; the SOCD + hat rewrite (above) supersedes it.
