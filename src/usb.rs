//! USB HID gamepad class.
//!
//! Mirrors the original `USB_DEVICE/App/usbd_custom_hid_if.c` and `Customer/gamepad.c`.
//! Report layout is 9 bytes; descriptor bytes are copied verbatim from the
//! ST-generated `CUSTOM_HID_ReportDesc_FS`.
//!
//! API aligned to embassy-stm32 0.3.0 + embassy-usb 0.5.0:

use embassy_stm32::usb::{Driver, InterruptHandler as UsbIrq};
use embassy_stm32::{bind_interrupts, peripherals};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    signal::Signal,
};
use embassy_usb::{
    class::hid::{Config as HidConfig, HidWriter, State as HidState},
    Builder, Config as UsbConfig, UsbDevice,
};
use static_cell::StaticCell;

/// 9-byte gamepad report, matching the original `uint8_t Buf[9]` in gamepad.c:140.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct GamepadReport {
    pub x: u8,
    pub y: u8,
    pub rx: u8,
    pub ry: u8,
    pub z: u8,
    pub buttons: u16,   // 10 bits; low 4 bits = ST|MD|BK|TB
    pub hat: u8,        // 1..=8, 0 = released
}

impl GamepadReport {
    pub const NEUTRAL: Self = Self {
        x: 128, y: 128, rx: 128, ry: 128, z: 128,
        buttons: 0, hat: 0,
    };
}

/// Business side signals a generated report here; the HID writer task consumes
/// it and writes to the interrupt EP.
pub static REPORT_TX: Signal<CriticalSectionRawMutex, GamepadReport> = Signal::new();

/// Verbatim copy of `CUSTOM_HID_ReportDesc_FS` from usbd_custom_hid_if.c:92.
pub const HID_REPORT_DESC: &[u8] = &[
    0x05, 0x01,        // USAGE_PAGE (Generic Desktop)
    0x09, 0x05,        // USAGE (Game Pad)
    0xa1, 0x01,        //   COLLECTION (Application)
    0xa1, 0x00,        //     COLLECTION (Physical)  — X / Y
    0x09, 0x30, 0x09, 0x31,
    0x15, 0x00, 0x26, 0xff, 0x00, 0x35, 0x00, 0x46, 0xff, 0x00,
    0x95, 0x02, 0x75, 0x08, 0x81, 0x02, 0xc0,
    0xa1, 0x00,        //     COLLECTION (Physical)  — Rx / Ry
    0x09, 0x33, 0x09, 0x34,
    0x15, 0x00, 0x26, 0xff, 0x00, 0x35, 0x00, 0x46, 0xff, 0x00,
    0x95, 0x02, 0x75, 0x08, 0x81, 0x02, 0xc0,
    0xa1, 0x00,        //     COLLECTION (Physical)  — Z
    0x09, 0x32,
    0x15, 0x00, 0x26, 0xff, 0x00, 0x35, 0x00, 0x46, 0xff, 0x00,
    0x95, 0x01, 0x75, 0x08, 0x81, 0x02, 0xc0,
    0x05, 0x09,        //   USAGE_PAGE (Button)
    0x19, 0x01, 0x29, 0x0a, 0x95, 0x0a, 0x75, 0x01, 0x81, 0x02,
    0x05, 0x01,        //   USAGE_PAGE (Generic Desktop)
    0x09, 0x39,        //   USAGE (Hat switch)
    0x15, 0x01, 0x25, 0x08, 0x35, 0x00, 0x46, 0x3b, 0x10, 0x66, 0x0e, 0x00,
    0x75, 0x04, 0x95, 0x01, 0x81, 0x42,
    0x75, 0x02, 0x95, 0x01, 0x81, 0x03,
    0x75, 0x08, 0x95, 0x02, 0x81, 0x03, 0xC0,
];

/// USB VID/PID kept from the original project (`usbd_desc.c`: VID=0x1234, PID=0xABCD).
pub const VID: u16 = 0x1234;
pub const PID: u16 = 0xABCD;

bind_interrupts!(pub struct UsbIrqs {
    USB_LP_CAN1_RX0 => UsbIrq<peripherals::USB>;
});

pub type UsbDriver = Driver<'static, peripherals::USB>;
pub type UsbHidWriter = HidWriter<'static, UsbDriver, 64>;

/// Called once from main. Returns (&'static mut HID writer, USB stack future).
///
/// All 'static state is held in StaticCell. Builder is constructed on the stack;
/// `HidWriter::new(&mut builder, ...)` borrows it, `builder.build()` consumes it,
/// the resulting `UsbDevice` is placed in a StaticCell and run() inside an async block.
pub fn build_usb(
    driver: UsbDriver,
) -> (&'static mut UsbHidWriter, impl core::future::Future<Output = ()>) {
    static HID_WRITER: StaticCell<UsbHidWriter> = StaticCell::new();
    static HID_STATE: StaticCell<HidState<'static>> = StaticCell::new();
    static USB_DEV: StaticCell<UsbDevice<'static, UsbDriver>> = StaticCell::new();

    // embassy-usb 0.5 Builder::new requires four descriptor buffers
    static CONFIG_DESC_BUF: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC_BUF:    StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESC_BUF:   StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF:     StaticCell<[u8; 64]>  = StaticCell::new();

    let mut config = UsbConfig::new(VID, PID);
    config.manufacturer = Some("LDSCITECHE");
    config.product = Some("MINIGPA103 Gamepad (rust)");
    config.serial_number = Some("rust-001");
    config.max_power = 50;                    // 100 mA
    config.max_packet_size_0 = 8;             // F103 USB FS control endpoint max 8 bytes

    // Builder constructed on stack, internally holds 'static buffer references
    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESC_BUF.init([0; 256]),
        BOS_DESC_BUF.init([0; 256]),
        MSOS_DESC_BUF.init([0; 256]),
        CONTROL_BUF.init([0; 64]),
    );

    // HidWriter::new(&mut builder, state, config) — business is write-only,
    // use HidWriter instead of HidReaderWriter
    let writer = HID_WRITER.init(HidWriter::new(
        &mut builder,
        HID_STATE.init(HidState::new()),
        HidConfig {
            report_descriptor: HID_REPORT_DESC,
            request_handler: None,
            poll_ms: 5,                       // bInterval, original 5ms
            max_packet_size: 64,
        },
    ));

    // builder.build() consumes builder, yields UsbDevice<'static>, stored in StaticCell
    let usb = USB_DEV.init(builder.build());

    (writer, async move {
        // UsbDevice::run(&mut self) -> impl Future<Output = !>
        // ! coerces to (), matching this future's Output
        usb.run().await;
    })
}
