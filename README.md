# gamepad-rs

MINIGPA103 USB HID 游戏手柄的 Rust + Embassy 重实现，路径 C 方案。

## 目标硬件

- APM32F103C8（STM32F103C8 寄存器兼容，Cortex-M3 @ 72 MHz, 64K Flash / 20K RAM）
- 原理图：`Doc/原理图/SCH_MINIGPV103_Beta_2025-03-22.pdf`
- 引脚映射（已从 `Core/Src/{adc,gpio,tim}.c` 核对）：

| 功能 | 引脚 | 外设 |
|---|---|---|
| 摇杆 X | PA1 | ADC1_IN1 |
| 摇杆 Y | PA2 | ADC1_IN2 |
| 按键 UP/DN/LF/RG | PB9 / PB8 / PB7 / PB6 | GPIO 上拉，低有效 |
| 按键 BK/MD/ST/TB | PB5 / PB4 / PB3 / PA15 | GPIO 上拉，低有效 |
| 按键 SW1 | PA0 | GPIO 下拉（Eg8 未用） |
| WS2812B DIN | PA6 | TIM3_CH1 (PWM 800kHz + DMA1_CH6) |
| USB D+/D- | PA12 / PA11 | USB full-speed device |

## 工具链

```bash
rustup target add thumbv7m-none-eabi
cargo install probe-rs   # 烧录/调试
```

## 构建

```bash
cd gamepad-rs
cargo build --release
```

## 烧录运行

```bash
cargo run --release          # 经 probe-rs 烧录并打开 defmt RTT 日志
```

调试器：ST-Link V2（SWD）。芯片名按调试器识别改 `.cargo/config.toml` 的 `--chip`。

## 模块对照

| 原工程文件 | 本项目 |
|---|---|
| `Core/Src/main.c` | `src/main.rs` |
| `Customer/gamepad.c` | `src/gamepad.rs` |
| `Customer/ws281x.c` | `src/ws2812.rs` |
| `USB_DEVICE/App/usbd_custom_hid_if.c` | `src/usb.rs` |
| `Customer/MultiTimer.c` | `embassy_time::Timer` 直接 await |
| `Core/Src/{adc,gpio,tim}.c` | `main.rs` 中的 `embassy_stm32` 初始化 |

## 状态 / TODO

- [x] 项目骨架（Cargo.toml / memory.x / build.rs / .cargo/config.toml）
- [x] USB HID class + 描述符照搬
- [x] gamepad 业务（ADC 均值 + hat 真值表 + 变化上报）
- [x] **首次编译通过**（embassy 0.3/0.5/0.7 实际 API 对齐完成）
- [x] defmt 1.0 `#[defmt::panic_handler]` 配置
- [x] 工具链：rustup stable MSVC + thumbv7m-none-eabi + probe-rs 0.31.0 + cargo-binutils
- [x] 尺寸确认：Flash 28.7KB / 64KB（45%），RAM 4.7KB / 20KB（24%）
- [ ] **WS2812B DMA 写 CCR**：`ws2812.rs::write_pixel_blocking` 当前是阻塞占位，时序不达标，需用 DMA1_CH6 连续送 24 个 CCR 值
- [ ] **PA15 / PB3 / PB4 JTAG 引脚**：embassy-stm32 是否自动 disable JTAG 待烧录验证
- [ ] **首次烧录实测**：ST-Link 接 SWD，`cargo run --release` 看是否识别为游戏手柄
- [ ] Eg10_Xinput（vendor-specific class）未实现，是后续独立任务

## 不修复的"原版 bug"

`gamepad.rs::hat_lookup` 完全照抄原 `gamepad.c:170-208` 的真值表，逻辑与方向不对应（按键组合和 hat 方向不匹配）。这是 1:1 复刻的取舍；想得到正确的方向响应需重写该表。
