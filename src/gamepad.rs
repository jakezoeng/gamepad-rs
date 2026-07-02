//! 游戏手柄业务逻辑 —— 对照 `Customer/gamepad.c`。
//!
//! 1. ADC 采样 10 次 X / 10 次 Y 求均值（原 `AD_DATA[20]` DMA 循环的等价物）。
//! 2. `map()` 映射到 0..=255，clamp 到 [Xmin+10, Xmax-10]。
//! 3. 按键扫描（低电平有效），按真值表算 Hat 值。
//! 4. 仅在报告变化时 signal 给 USB writer 任务（原 gamepad.c:235 的 memcmp）。

use embassy_stm32::adc::Adc;
use embassy_stm32::gpio::{Input, Pull};
use embassy_stm32::peripherals::{ADC1, PA1, PA15, PB3, PB4, PB5, PB6, PB7, PB8, PB9, PA2};
use embassy_stm32::Peri;
use embassy_time::Timer;

use crate::usb::{GamepadReport, REPORT_TX};

// 原 gamepad.h:65-68
const AD_XMIN: i32 = 0;
const AD_XMAX: i32 = 0xfc1;   // 4033
const AD_YMIN: i32 = 0;
const AD_YMAX: i32 = 0xfc1;

// 原 gamepad.h:56-63
const HAT_N: u8 = 0x00;
const HAT_1: u8 = 0x04;
const HAT_2: u8 = 0x08;
const HAT_3: u8 = 0x0C;
const HAT_4: u8 = 0x10;
const HAT_5: u8 = 0x14;
const HAT_6: u8 = 0x18;
const HAT_7: u8 = 0x1C;
const HAT_8: u8 = 0x20;

/// 原 gamepad.c:128
fn map(x: i32, in_min: i32, in_max: i32, out_min: i32, out_max: i32) -> i32 {
    (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
}

#[inline]
fn pressed(p: &Input<'static>) -> bool {
    p.is_low()
}

/// 4 个方向键 → Hat 值，完全照搬 gamepad.c:170-208 的真值表。
///
/// `(UP, DN, LF, RG)` 真值表，1 = 按下。原代码逻辑混乱，本文件 1:1 复刻不修正。
fn hat_lookup(up: bool, dn: bool, lf: bool, rg: bool) -> u8 {
    match (up, dn, lf, rg) {
        (true,  true,  true,  false) => HAT_7,
        (true,  true,  false, true)  => HAT_3,
        (true,  false, true,  false) => HAT_1,
        (true,  false, true,  true)  => HAT_8,
        (true,  false, false, false) => HAT_2,
        (true,  false, false, true)  => HAT_1,
        (false, true,  true,  false) => HAT_5,
        (false, true,  true,  true)  => HAT_6,
        (false, true,  false, false) => HAT_4,
        (false, true,  false, true)  => HAT_5,
        (false, false, true,  true)  => HAT_7,
        (false, false, true,  false) => HAT_3,
        _ => HAT_N,
    }
}

/// gamepad 任务。参数为具体 peripheral 类型（embassy-stm32 0.3 的 `Peri<'static, _>`）。
#[embassy_executor::task]
pub async fn gamepad_task(
    mut adc: Adc<'static, ADC1>,
    mut pin_x: Peri<'static, PA1>,    // ADC1_IN1 = X
    mut pin_y: Peri<'static, PA2>,    // ADC1_IN2 = Y
    up:  Peri<'static, PB9>,
    dn:  Peri<'static, PB8>,
    lf:  Peri<'static, PB7>,
    rg:  Peri<'static, PB6>,
    bk:  Peri<'static, PB5>,
    md:  Peri<'static, PB4>,
    st:  Peri<'static, PB3>,
    tb:  Peri<'static, PA15>,
) {
    // PA1/PA2 已被 AdcChannel impl（无需手动 set_as_analog，read() 会调 setup()）。
    // 但需把它们从默认 GPIO 模式释放给 ADC —— embassy-stm32 的 impl_adc_pin! 在
    // setup() 里会调 set_as_analog。我们只需保证这些 pin 不被别的驱动占用即可。
    // 这里 pin_x/pin_y 不需要构造 Input，直接作为 channel 传给 read()。

    // 按键：上拉输入（原 gpio.c:60/68）
    let up = Input::new(up, Pull::Up);
    let dn = Input::new(dn, Pull::Up);
    let lf = Input::new(lf, Pull::Up);
    let rg = Input::new(rg, Pull::Up);
    let bk = Input::new(bk, Pull::Up);
    let md = Input::new(md, Pull::Up);
    let st = Input::new(st, Pull::Up);
    let tb = Input::new(tb, Pull::Up);

    let mut last = GamepadReport::NEUTRAL;

    loop {
        // —— ADC 均值：原 AD_DATA[20] 是 X/Y 交替 10 组，等价于 10 次均值
        let (mut sx, mut sy) = (0u32, 0u32);
        for _ in 0..10 {
            sx += adc.read(&mut pin_x).await as u32;
            sy += adc.read(&mut pin_y).await as u32;
        }
        let xt = (sx / 10) as i32;
        let yt = (sy / 10) as i32;

        let xt = xt.clamp(AD_XMIN + 10, AD_XMAX - 10);
        let yt = yt.clamp(AD_YMIN + 10, AD_YMAX - 10);

        // 原 gamepad.c:166-167: Buf[1]=map(X), Buf[0]=map(Y)
        let x = map(xt, AD_XMIN + 10, AD_XMAX - 10, 0, 255) as u8;
        let y = map(yt, AD_YMIN + 10, AD_YMAX - 10, 0, 255) as u8;

        // —— 按键扫描（原 gamepad.c:170+）
        let (u, d, l, r) = (pressed(&up), pressed(&dn), pressed(&lf), pressed(&rg));
        let hat = hat_lookup(u, d, l, r);

        // Buf[5] bit0..3 = ST|MD|BK|TB（原 gamepad.c:210-233）
        let buttons: u16 = (pressed(&st) as u16) << 0
                         | (pressed(&md) as u16) << 1
                         | (pressed(&bk) as u16) << 2
                         | (pressed(&tb) as u16) << 3;

        let now = GamepadReport { x, y, rx: 128, ry: 128, z: 128, buttons, hat };

        // 仅变化才上报（原 gamepad.c:235 memcmp）
        if now != last {
            REPORT_TX.signal(now);
            last = now;
        }
        Timer::after_millis(5).await;   // 原 timer1 5ms
    }
}

// 抑制未使用警告
#[allow(dead_code)]
fn _unused() {}
