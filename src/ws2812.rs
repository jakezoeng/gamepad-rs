//! WS2812B via TIM3_CH1（对照 `Customer/ws281x.c` + `Core/Src/tim.c`）。
//!
//! 原工程参数：
//!   - TIM3 时钟源 = APB1 ×2 = 72 MHz
//!   - Prescaler=0, Period=89 → PWM 周期 = 72M / 90 = 800 kHz (1.25µs)
//!   - WS_HIGH = 57, WS_LOW = 22
//!   - DMA1_Channel6, Memory→Peripheral, HalfWord, Normal 模式
//!
//! 当前实现：**阻塞占位**，时序不达标但能编译通过。
//! 真正能用必须用 DMA 写 CCR1。TODO: 用 timer::low_level 或裸寄存器配 DMA。

use embassy_stm32::timer::simple_pwm::{SimplePwm, SimplePwmChannel};
use embassy_stm32::timer::GeneralInstance4Channel;
use embassy_stm32::{peripherals::TIM3, time::Hertz};
use embassy_time::Timer;
use smart_leds::RGB8;

const WS_HIGH: u16 = 57;        // 原 ws281x.h:18
const WS_LOW:  u16 = 22;        // 原 ws281x.h:19

/// WS2812B 驱动占位 —— 持有 SimplePwm，每次写像素时临时取 ch1。
///
/// **时序不达标**：阻塞 set_duty 无法产生连续 800kHz 比特流。
/// 真正实现需要 DMA。这里只为让项目编译通过。
pub struct Ws2812 {
    pub pwm: SimplePwm<'static, TIM3>,
}

impl Ws2812 {
    pub fn new(pwm: SimplePwm<'static, TIM3>) -> Self {
        Self { pwm }
    }
}/// 彩虹任务，对照 `ws281x.c:89 WS281x_Rainbow` + `main.c:86 WS2812BTimer2Callback`。
#[embassy_executor::task]
pub async fn ws2812_rainbow(mut led: Ws2812) {
    let mut j: u8 = 0;
    loop {
        let c = wheel(j);
        // 占位：用 ch1 临时写。时序不准，WS2812B 实际不会正确响应。
        let mut ch = led.pwm.ch1();
        ch.enable();
        let _ = write_pixel_blocking(&mut ch, c);
        j = j.wrapping_add(1);
        Timer::after_millis(100).await;     // 原 timer2 100ms
    }
}

/// 阻塞写一个像素 —— 时序不准，仅占位。
fn write_pixel_blocking<T: GeneralInstance4Channel>(
    ch: &mut SimplePwmChannel<'_, T>,
    c: RGB8,
) -> Result<(), ()> {
    // TODO: 改成 DMA 写 24 个 CCR 值的 buffer
    for byte in [c.g, c.r, c.b] {
        for bit in (0..8).rev() {
            let duty = if (byte >> bit) & 1 == 1 { WS_HIGH } else { WS_LOW };
            ch.set_duty_cycle(duty);
            // 这里没有精确 1.25µs 延时，WS2812B 实际不会正确响应。
        }
    }
    ch.set_duty_cycle(0);   // reset
    Ok(())
}

/// 原 `ws281x.c:62 WS281x_Wheel` —— 输入 0..255，输出彩虹色。
fn wheel(wheel_pos: u8) -> RGB8 {
    let wheel_pos = 255 - wheel_pos;
    if wheel_pos < 85 {
        RGB8::new(255 - wheel_pos * 3, 0, wheel_pos * 3)
    } else if wheel_pos < 170 {
        let wheel_pos = wheel_pos - 85;
        RGB8::new(0, wheel_pos * 3, 255 - wheel_pos * 3)
    } else {
        let wheel_pos = wheel_pos - 170;
        RGB8::new(wheel_pos * 3, 255 - wheel_pos * 3, 0)
    }
}

#[allow(dead_code)]
fn _unused(_: Hertz) {}
