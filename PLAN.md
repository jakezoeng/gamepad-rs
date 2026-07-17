# gamepad-rs 配置上位机 + OTA 方案

> 用户需求：参考 QMK 上位机（VIA/Toolbox 风格：桌面 GUI，配置 + 烧录一体），固件 + 上位机一起做，OTA 走 App 内置 DFU（HID 传输进 bootloader），配置项全要（摇杆校准 / 按键映射 / SOCD / Turbo / LED）。

## 一、现状评估

| 维度 | 现状 | 差距 |
|---|---|---|
| USB | 单 HID 接口，仅 9 字节 gamepad 输出 | 无配置通道、无 DFU |
| Flash 持久化 | 无 | 配置掉电丢失 |
| Flash 占用 | 28.7 KB / 64 KB (45%) | 余量 ~35 KB，但需切出 16 KB bootloader |
| 内存布局 | 单段 App `0x08000000-0x0800FFFF` | 需拆 Bootloader / App / Config |
| WS2812 | 阻塞占位，时序不合规 | LED 配置功能前置依赖 |
| hat_lookup | 1:1 复刻原版 bug | SOCD 重写替换 |

**关键约束**：STM32F103C8 是**单 bank flash**，运行中的代码不能擦自身 —— DFU 必须由独立 bootloader 执行；App 只负责"设标志 + 重启进 bootloader"。

## 二、目标架构

```
┌─────────────────────────────────────────────────────────┐
│  Host (Tauri + Rust)                                    │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │ 设备枚举     │  │ 配置 UI       │  │ DFU 升级 UI    │  │
│  │ (hidapi)    │  │ 实时预览      │  │ 进度/校验      │  │
│  └──────┬──────┘  └──────┬───────┘  └──────┬────────┘  │
│         └────────────────┴─────────────────┘            │
│                     gamepad-host (lib)                   │
│                     gamepad-protocol (shared, no-std)    │
└────────────────────────────┬────────────────────────────┘
                             │ USB HID (VID 0x1234 PID 0xABCD)
                             │  IF0 gamepad / IF1 vendor
┌────────────────────────────┴────────────────────────────┐
│  Device (STM32F103C8)                                   │
│  ┌──────────────────────────────────────────────────┐   │
│  │ Bootloader @ 0x08000000 (16 KB)                  │   │
│  │  - 检查 reboot flag → 进 DFU 或跳 App            │   │
│  │  - DFU: HID 收包 → 擦写 App 区 → 校验 → 跳转      │   │
│  │  - 公钥预留位（签名验证后续接入）                  │   │
│  ├──────────────────────────────────────────────────┤   │
│  │ App @ 0x08004000 (~47 KB)                        │   │
│  │  - IF0 Gamepad HID (现状)                        │   │
│  │  - IF1 Vendor HID: 配置 + DFU 控制               │   │
│  │  - SOCD / Remap / Turbo / 校准 / LED 业务        │   │
│  ├──────────────────────────────────────────────────┤   │
│  │ Config @ 0x0800FC00 (1 KB, 末页)                 │   │
│  │  - magic + Config struct + CRC32                 │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

## 三、USB 接口设计

**IF0 — Gamepad HID**（保持现状，9 字节报告，5ms bInterval）

**IF1 — Vendor HID**（新增，配置 + DFU 控制）
- `report_descriptor`：generic vendor page 0xFF00，64-byte Feature Report IN/OUT，Interrupt EP IN 64 字节
- 上位机用 `hidapi` 的 `send_feature_report` / `read_feature_report` 或 `write` / `read_interrupt`
- 所有命令包定长 64 字节，避免短包问题

## 四、配置协议（共享 crate `gamepad-protocol`）

`no_std` + `serde`（`serde` core 子集），固件和上位机共用，杜绝协议漂移。

### 4.1 命令码

| Opcode | 名称 | 方向 | 说明 |
|---|---|---|---|
| 0x01 | `GetProtocolVersion` | H→D→H | 协议版本，防不兼容 |
| 0x02 | `GetDeviceInfo` | H→D→H | fw 版本 / build / flash 大小 / bootloader 版本 |
| 0x03 | `GetConfig` | H→D→H | 返回当前 Config 结构 |
| 0x04 | `SetConfig` | H→D | 写 RAM（暂存），不落 flash |
| 0x05 | `SaveConfig` | H→D→H | 擦写 Config 页 + CRC，返回 ok/err |
| 0x06 | `ResetConfig` | H→D→H | 恢复出厂默认 |
| 0x10 | `GetLiveReport` | H→D→H | 实时取一份 gamepad report（预览用） |
| 0x20 | `EnterDFU` | H→D | 设 reboot flag + 软复位 |
| 0x21 | `DFUHandshake` | H→D→H | bootloader 握手（App 不响应） |
| 0x22 | `DFUErasePage` | H→D→H | 擦指定页 |
| 0x23 | `DFUWriteChunk` | H→D→H | 写 56 字节到指定地址 |
| 0x24 | `DFUVerify` | H→D→H | CRC32 校验整段 App 区 |
| 0x25 | `DFUReboot` | H→D | 跳 App |

### 4.2 Config 结构（serde + 固定布局）

```rust
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Config {
    pub magic: u32,                 // 0x47504346 "GPCF"
    pub version: u16,               // 结构版本号，升级时迁移
    pub socd: SocdMode,             // UpPriority / Neutral / LastInput
    pub poll_ms: u8,                // 1..=10
    pub joystick: JoystickCfg,
    pub buttons: ButtonCfg,
    pub turbo: TurboCfg,
    pub led: LedCfg,
    pub crc: u32,                   // 末尾 CRC32（除 crc 字段外）
}

pub struct JoystickCfg {
    pub x_min: u16, pub x_max: u16, pub x_dead: u16,
    pub y_min: u16, pub y_max: u16, pub y_dead: u16,
    pub invert_x: bool, pub invert_y: bool,
    pub dpad_mode: DpadMode,        // Dpad / LeftStick / RightStick
}

pub struct ButtonCfg {
    pub remap: [u8; 10],            // physical → logical button index
}

pub struct TurboCfg {
    pub enabled: [bool; 10],        // 每键独立
    pub hz: u8,                     // 5..=20
}

pub struct LedCfg {
    pub mode: LedMode,              // Off / Static / Rainbow / Breathing
    pub color: [u8; 3],             // RGB
    pub brightness: u8,             // 0..=255
}
```

Flash 落盘格式：直接把 `Config` 序列化（postcard，~80 字节），写 1 页即可，末尾 CRC32 校验。启动时如 magic 不符或 CRC 错 → 加载默认。

## 五、仓库结构（改 workspace）

```
gamepad-rs/
├── Cargo.toml                  # [workspace]，列出下方成员
├── memory.x                    # firmware 用
├── firmware/
│   ├── Cargo.toml              # no_std crate, 现在的 src/ 全搬过来
│   ├── build.rs
│   ├── memory.x
│   └── src/
│       ├── main.rs             # App 入口
│       ├── usb.rs              # 双接口
│       ├── config_iface.rs     # IF1 vendor HID 处理
│       ├── config_store.rs     # flash 读写 + CRC
│       ├── gamepad.rs          # 加 SOCD/Remap/Turbo
│       ├── socd.rs             # 新
│       ├── turbo.rs            # 新
│       └── ws2812.rs           # DMA 重写
├── bootloader/
│   ├── Cargo.toml
│   ├── memory.x                # 仅 16 KB
│   └── src/main.rs             # DFU 接收 + 擦写 + 跳转
├── crates/
│   └── protocol/
│       ├── Cargo.toml          # no_std + serde, 固件/上位机共用
│       └── src/lib.rs
└── host/
    ├── core/                   # gamepad-host lib (hidapi + 协议)
    │   ├── Cargo.toml
    │   └── src/lib.rs
    └── gui/                    # Tauri 2.x
        ├── Cargo.toml
        ├── src-tauri/
        └── ui/                 # 前端（React + Vite 或 Svelte）
```

> **理由**：当前根 `Cargo.toml` 是 firmware 包，要保留可单独 `cargo build --release` 烧录的能力，最干净的办法是转 workspace，把 firmware 移到子目录。Bootloader 独立 crate 独立 binary，不与 App 共享链接脚本。

## 六、分阶段交付（里程碑）

### M1 — 配置闭环（固件 + 上位机）
**目标**：上位机能连设备、读写配置、实时预览，配置掉电不丢。不含 OTA、不含 LED（LED 受 WS2812 阻塞）。

固件侧：
1. 转 workspace，搬 firmware 到子目录
2. 新建 `crates/protocol`，定义 `Config` + 命令枚举 + postcard 编解码
3. firmware 引入 protocol crate
4. `usb.rs`：双接口构建（IF0 gamepad / IF1 vendor）
5. `config_iface.rs`：IF1 命令分发（Get/Set/Save/Reset/GetLiveReport）
6. `config_store.rs`：flash 末页读写 + CRC，启动加载
7. `socd.rs`：UpPriority / Neutral / LastInput，替换 `hat_lookup`
8. `gamepad.rs`：加 button remap、turbo 计数器、dpad_mode 切换、摇杆校准应用
9. `main.rs`：启动时加载 Config，注入 gamepad_task

上位机侧：
1. `host/core`：hidapi 封装，设备枚举（VID/PID 过滤）、连接、命令收发、超时
2. `host/gui`：Tauri 2 项目脚手架
3. UI：设备连接状态、Tab（摇杆 / 按键 / SOCD / Turbo）、实时预览画布、保存/重置按钮
4. Profile 导入导出（JSON，本地）

### M2 — Bootloader + DFU
**目标**：上位机选 `.bin` 文件，点"升级"，App 设标志重启，bootloader 接管擦写校验，跳回新 App。

固件侧：
1. `bootloader/`：独立 crate，memory.x 限 16 KB
2. Bootloader 逻辑：读 BKP 寄存器 / 特定 RAM 地址的 reboot flag
   - flag == 0：跳 `0x08004000` App
   - flag == `0xDF5A5ADF`：初始化 USB，监听 DFU 命令
3. DFU 流程：`DFUHandshake` → 多次 `DFUErasePage` → 多次 `DFUWriteChunk` → `DFUVerify` → `DFUReboot`
4. App 侧 `EnterDFU`：写 flag 到 BKP 寄存器（需 PWR 时钟使能）+ NVIC_SystemReset
5. App 的 `memory.x` 改 ORIGIN=0x08004000，向量表 `SCB_VTOR` 重定向
6. App 输出 `.bin`（cargo-binutils `cargo objcopy -- -O binary`）

上位机侧：
1. 固件升级 Tab：文件选择（.bin）、版本对比、进度条
2. DFU 状态机：EnterDFU → 等待重连（VID/PID 可能改）→ Handshake → 按页擦写 → Verify → Reboot
3. 失败回滚提示（bootloader 拒绝跳坏 App，提示用户重试）

### M3 — LED + 校准向导（依赖 WS2812 修复）
固件侧：
1. `ws2812.rs` DMA 重写：用 timer low_level 配 DMA1_CH6，24 × CCR 值缓冲
2. `LedCfg` 接入：Static / Rainbow / Breathing 三种模式
3. 摇杆校准向导协议：`StartCalibration` / `GetCalibMinMax` / `CommitCalib`

上位机侧：
1. LED Tab：颜色拾取、亮度滑块、模式切换
2. 摇杆校准向导：3 步（中心 → 角落 → 死区），实时显示原始 ADC 值

## 七、技术选型

| 组件 | 选型 | 理由 |
|---|---|---|
| 上位机框架 | **Tauri 2.x** | 单文件 ~10MB、原生 HID、Web UI（参考 VIA 风格）、Rust 后端与固件同语言 |
| 前端 | **Svelte + Vite** | 体积小、响应式适合实时数据；备选 React |
| HID 库 | `hidapi` 2.x | 跨平台，Windows 用 hid.dll，macOS/Linux 兼容 |
| 协议序列化 | `postcard` | no_std 友好、紧凑、`serde` 派生 |
| 配置文件 | JSON (host) + postcard (flash) | 用户可读 profile / 设备紧凑存储 |
| DFU 签名 | 本期不做 | 预留 opcode，README #12 落地后接入 |

## 八、风险与对策

| 风险 | 对策 |
|---|---|
| 64 KB Flash 偏紧，bootloader 16 KB + 协议代码 + 业务可能溢出 | M1 先测 App 大小，若超 40 KB 先优化（去 defmt release 版本 / `opt-level=z`） |
| 单 bank flash：App 升级时若断电会变砖 | Bootloader 永远不被擦写；App 升级失败可重试，bootloader 兜底 |
| STM32F103 BKP 寄存器需要 PWR 时钟 + LSE 才能掉电保持，但软复位能保 | 仅需软复位保 flag 即可，不需 LSE；用 BKP DR1 写 magic |
| WS2812 DMA 在 embassy-stm32 0.3 里 API 不直观 | M3 单独花时间，必要时降级到寄存器层 |
| Windows HID 排他访问：IF0 被 gamepad API 占用可能影响 IF1 | IF1 用独立 interface + 独立 collection，Windows 会分给不同 device path；必要时禁用 IF0 的系统占用 |
| 协议演进导致旧固件不识别新上位机 | `GetProtocolVersion` 握手，不匹配拒绝配置并提示升级 |

## 九、本期明确不做

- Ed25519 签名验证（README #12，独立 issue）
- RDP Level 2 锁定（README #14，发货前最后一脚）
- SBOM / CRA 文档（README #15）
- Web 版配置器（用户要桌面 GUI）
- Xinput / 多主机协议切换（README Eg10，独立任务）

## 十、需要用户确认的开放项

1. **前端框架**：Svelte（轻量）还是 React（生态大）？默认 Svelte。
2. **Profile 文件位置**：`%APPDATA%/gamepad-rs/profiles/`？默认是。
3. **Bootloader 烧录方式**：首次出厂用 ST-Link 烧 bootloader + app；之后 App 升级走 USB。是否接受"首次必须 ST-Link"？
4. **WS2812 LED 数量**：原理图只画了 1 颗，是否就按 1 颗？
5. **是否需要"恢复出厂"硬件按键组合**（如 Start+Select+Up 长按 3 秒清 Config）？建议加，作为 OTA 失败兜底之外的配置兜底。
