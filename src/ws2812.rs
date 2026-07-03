//! WS2812B via TIM3_CH1 (mirrors `Customer/ws281x.c` + `Core/Src/tim.c`).
//!
//! Original project parameters:
//!   - TIM3 clock source = APB1 ×2 = 72 MHz
//!   - Prescaler=0, Period=89 → PWM period = 72M / 90 = 800 kHz (1.25µs)
//!   - WS_HIGH = 57, WS_LOW = 22
//!   - DMA1_Channel6, Memory→Peripheral, HalfWord, Normal mode
//!
//! Current implementation: **blocking placeholder**, timing non-compliant
//! but compiles. Real implementation requires DMA write to CCR1.
//! TODO: use timer::low_level or raw registers to configure DMA.

use embassy_stm32::timer::simple_pwm::{SimplePwm, SimplePwmChannel};
use embassy_stm32::timer::GeneralInstance4Channel;
use embassy_stm32::{peripherals::TIM3, time::Hertz};
use embassy_time::Timer;
use smart_leds::RGB8;

const WS_HIGH: u16 = 57;        // original ws281x.h:18
const WS_LOW:  u16 = 22;        // original ws281x.h:19

/// WS2812B driver placeholder — holds SimplePwm, borrows ch1 per pixel write.
///
/// **Timing non-compliant**: blocking set_duty cannot produce a continuous
/// 800 kHz bitstream. Real implementation needs DMA. This exists only to
/// make the project compile.
pub struct Ws2812 {
    pub pwm: SimplePwm<'static, TIM3>,
}

impl Ws2812 {
    pub fn new(pwm: SimplePwm<'static, TIM3>) -> Self {
        Self { pwm }
    }
}

/// Rainbow task, mirrors `ws281x.c:89 WS281x_Rainbow` + `main.c:86 WS2812BTimer2Callback`.
#[embassy_executor::task]
pub async fn ws2812_rainbow(mut led: Ws2812) {
    let mut j: u8 = 0;
    loop {
        let c = wheel(j);
        // Placeholder: borrow ch1 for each write. Timing non-compliant;
        // WS2812B will not respond correctly.
        let mut ch = led.pwm.ch1();
        ch.enable();
        let _ = write_pixel_blocking(&mut ch, c);
        j = j.wrapping_add(1);
        Timer::after_millis(100).await;     // original timer2 100ms
    }
}

/// Blocking write of a single pixel — timing non-compliant, placeholder only.
fn write_pixel_blocking<T: GeneralInstance4Channel>(
    ch: &mut SimplePwmChannel<'_, T>,
    c: RGB8,
) -> Result<(), ()> {
    // TODO: replace with DMA write of a 24-CCR-value buffer
    for byte in [c.g, c.r, c.b] {
        for bit in (0..8).rev() {
            let duty = if (byte >> bit) & 1 == 1 { WS_HIGH } else { WS_LOW };
            ch.set_duty_cycle(duty);
            // No precise 1.25µs delay here; WS2812B will not respond correctly.
        }
    }
    ch.set_duty_cycle(0);   // reset
    Ok(())
}

/// Original `ws281x.c:62 WS281x_Wheel` — input 0..255, output rainbow color.
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
