//! MINIGPA103 USB HID 游戏手柄 —— Rust + Embassy 实现。
//!
//! 对照原 `Eg8_Gamepad/Core/Src/main.c`：
//!   - SystemClock_Config (HSE 8M × PLL9 = 72M, USB = 48M)
//!   - MX_GPIO_Init / MX_ADC1_Init / MX_USB_DEVICE_Init / MX_TIM3_Init
//!   - 两个 MultiTimer：timer1 (5ms) → Gamepad_Handle
//!                       timer2 (100ms) → WS281x_Rainbow
//!
//! API 对齐 embassy-stm32 0.3.0 + embassy-usb 0.5.0 + embassy-executor 0.7.0

#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;

/// defmt 1.0 需要用户用 `#[defmt::panic_handler]` 提供 `_defmt_panic` 符号。
/// panic-probe 1.0 文档推荐：用 `hard_fault()` 避免重复打印。
#[defmt::panic_handler]
fn panic() -> ! {
    panic_probe::hard_fault();
}

use embassy_executor::Spawner;
use embassy_stm32::adc::Adc;
use embassy_stm32::gpio::OutputType;
use embassy_stm32::rcc;
use embassy_stm32::timer::simple_pwm::{PwmPin, SimplePwm};
use embassy_stm32::timer::low_level::CountingMode;
use embassy_stm32::time::Hertz;
use embassy_stm32::Config;
use embassy_time::Timer;

mod gamepad;
mod usb;
mod ws2812;

/// HID writer 任务：消费 REPORT_TX signal，写 interrupt EP。
#[embassy_executor::task]
async fn hid_writer_task(writer: &'static mut usb::UsbHidWriter) {
    loop {
        let r = usb::REPORT_TX.wait().await;
        let buf: [u8; 9] = pack_report(r);
        let _ = writer.write(&buf).await;   // write(&mut self)，writer: &mut 自动 deref
    }
}

/// 按原 gamepad.c:140 的 `uint8_t Buf[9]` 字节布局打包。
fn pack_report(r: usb::GamepadReport) -> [u8; 9] {
    let mut b = [0u8; 9];
    b[0] = r.y;
    b[1] = r.x;
    b[2] = r.rx;
    b[3] = r.ry;
    b[4] = r.z;
    b[5] = (r.buttons & 0xFF) as u8;
    b[6] = r.hat;
    b[7] = 0;
    b[8] = 0;
    b
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // —— SystemClock_Config（对照 main.c:149）
    // F1 RCC Config 字段（embassy-stm32 0.3, src/rcc/f013.rs）
    let mut cfg = Config::default();
    cfg.rcc.hsi = false;
    cfg.rcc.hse = Some(rcc::Hse {
        freq: Hertz(8_000_000),
        mode: rcc::HseMode::Oscillator,
    });
    cfg.rcc.sys = rcc::Sysclk::PLL1_P;          // F1 变体名
    cfg.rcc.pll = Some(rcc::Pll {
        src: rcc::PllSource::HSE,
        prediv: rcc::PllPreDiv::DIV1,
        mul: rcc::PllMul::MUL9,                  // 8M × 9 = 72M
    });
    cfg.rcc.ahb_pre = rcc::AHBPrescaler::DIV1;
    cfg.rcc.apb1_pre = rcc::APBPrescaler::DIV2;  // 36M
    cfg.rcc.apb2_pre = rcc::APBPrescaler::DIV1;  // 72M
    cfg.rcc.adc_pre = rcc::ADCPrescaler::DIV6;   // ADC = 72/6 = 12M

    let p = embassy_stm32::init(cfg);
    defmt::info!("MINIGPA103 gamepad-rs boot");

    // —— MX_ADC1_Init
    let adc = Adc::new(p.ADC1);

    // —— MX_TIM3_Init (TIM3_CH1 = PA6, PWM 800kHz for WS2812B)
    let pwm = SimplePwm::new(
        p.TIM3,
        Some(PwmPin::new(p.PA6, OutputType::PushPull)),
        None, None, None,
        Hertz(800_000),     // 800 kHz，对应原 Period=89 @72MHz
        CountingMode::EdgeAlignedUp,
    );
    let led = ws2812::Ws2812::new(pwm);

    // —— MX_USB_DEVICE_Init
    let driver = embassy_stm32::usb::Driver::new(p.USB, usb::UsbIrqs, p.PA12, p.PA11);
    let (writer, usb_fut) = usb::build_usb(driver);

    // —— spawn 任务
    spawner.spawn(hid_writer_task(writer)).unwrap();
    spawner.spawn(gamepad::gamepad_task(
        adc,
        p.PA1, p.PA2,
        p.PB9, p.PB8, p.PB7, p.PB6, p.PB5, p.PB4, p.PB3, p.PA15,
    )).unwrap();
    spawner.spawn(ws2812::ws2812_rainbow(led)).unwrap();

    // —— USB 栈在 main 里跑
    usb_fut.await;

    loop { Timer::after_secs(60).await; }
}
