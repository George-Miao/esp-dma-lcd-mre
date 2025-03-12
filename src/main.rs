#![no_std]
#![no_main]
#![allow(clippy::unusual_byte_groupings)]

extern crate alloc;

use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    dma::DmaDescriptor,
    gpio::{Flex, Level, Output},
    lcd_cam::{
        lcd::{dpi::*, *},
        *,
    },
    time::Rate,
    xtensa_lx_rt::entry,
};
use log::info;
use static_cell::ConstStaticCell;

mod display;
mod dma;

use crate::{
    display::st7701::{ManualSpi, St7701},
    dma::DmaTxStreamBuf,
};

const MAX_RED: u16 = (1 << 5) - 1;

const RED: u16 = rgb(MAX_RED, 0, 0);

const fn rgb(r: u16, g: u16, b: u16) -> u16 {
    (r << 11) | (g << 5) | b
}

const V_RES: usize = 480;
const H_RES: usize = 480;

static DESCRIPTORS: ConstStaticCell<[DmaDescriptor; 100]> =
    ConstStaticCell::new([DmaDescriptor::EMPTY; 100]);

static BUFFER: ConstStaticCell<[u8; 100_000]> = ConstStaticCell::new([0; 100_000]);

#[entry]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();
    esp_alloc::heap_allocator!(10 * 1024);

    let peripherals: esp_hal::peripherals::Peripherals =
        esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let rst = Output::new(peripherals.GPIO47, Level::High, Default::default());
    let cs = Output::new(peripherals.GPIO21, Level::Low, Default::default());
    let scl = Output::new(peripherals.GPIO14, Level::Low, Default::default());
    let mut sda = Flex::new(peripherals.GPIO13);

    sda.set_as_output();

    let spi = ManualSpi { cs, sda, scl };

    let mut st7701 = St7701::new(spi, rst);
    let mut delay = Delay::new();

    info!("Initializing LCD");

    delay.delay_millis(50);

    st7701.init(&mut delay).unwrap();

    info!("Initialized");

    delay.delay_millis(50);

    let lcd_cam = LcdCam::new(peripherals.LCD_CAM);
    let channel = peripherals.DMA_CH0;

    let config = dpi::Config::default()
        .with_frequency(Rate::from_mhz(12))
        .with_clock_mode(ClockMode {
            polarity: Polarity::IdleLow,
            phase: Phase::ShiftHigh,
        })
        .with_format(Format {
            enable_2byte_mode: true,
            bit_order: BitOrder::Inverted,
            ..Default::default()
        })
        .with_timing(FrameTiming {
            horizontal_active_width: H_RES,
            horizontal_total_width: 500,
            horizontal_blank_front_porch: 10,

            vertical_active_height: V_RES,
            vertical_total_height: 493,
            vertical_blank_front_porch: 2,

            hsync_width: 10,
            vsync_width: 10,

            hsync_position: 0,
        })
        .with_vsync_idle_level(Level::High)
        .with_hsync_idle_level(Level::High)
        .with_de_idle_level(Level::Low)
        .with_disable_black_region(false);

    let dpi = Dpi::new(lcd_cam.lcd, channel, config)
        .unwrap()
        // Blue
        .with_data0(peripherals.GPIO46)
        .with_data1(peripherals.GPIO9)
        .with_data2(peripherals.GPIO10)
        .with_data3(peripherals.GPIO11)
        .with_data4(peripherals.GPIO12)
        // Green
        .with_data5(peripherals.GPIO17)
        .with_data6(peripherals.GPIO18)
        .with_data7(peripherals.GPIO8)
        .with_data8(peripherals.GPIO19)
        .with_data9(peripherals.GPIO20)
        .with_data10(peripherals.GPIO3)
        // Red
        .with_data11(peripherals.GPIO5)
        .with_data12(peripherals.GPIO6)
        .with_data13(peripherals.GPIO7)
        .with_data14(peripherals.GPIO15)
        .with_data15(peripherals.GPIO16)
        // Control
        .with_pclk(peripherals.GPIO40)
        .with_hsync(peripherals.GPIO39)
        .with_vsync(peripherals.GPIO38)
        .with_de(peripherals.GPIO37);

    let mut dma_buf = DmaTxStreamBuf::new(DESCRIPTORS.take(), BUFFER.take()).unwrap();

    loop {
        if dma_buf.push(&RED.to_be_bytes()) < 2 {
            break;
        }
    }

    let mut buffer = [0; 480 * 16];

    log::info!("Buffering");

    for chunk in buffer.chunks_mut(2) {
        let color: u16 = 0b11111_000000_00000;
        chunk.copy_from_slice(&color.to_le_bytes());
    }

    log::info!("Rendering");

    let mut transfer = dpi.send(true, dma_buf).map_err(|e| e.0).unwrap();

    // Uncomment this line and DMA will hang
    // esp_hal::delay::Delay::new().delay_millis(10);

    loop {
        transfer.push(&buffer, false);
    }
}
