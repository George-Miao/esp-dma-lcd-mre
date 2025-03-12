use core::convert::Infallible;

use embedded_hal::delay::DelayNs;
use esp_backtrace as _;
use esp_hal::{
    DriverMode,
    delay::Delay,
    gpio::{Flex, Output},
    spi::{
        DataMode, Error,
        master::{Address, Command, Spi},
    },
};

const MSB_MASK: u8 = 0b1000_0000;

fn ser(is_command: bool, byte: u8) -> Command {
    // First bit: 0 for command, 1 for parameter
    let first_bit = (!is_command as u16) << 15;
    // 1-bit C/D followed by 8-bit data
    let data = (byte as u16) << 7 | first_bit;

    Command::_9Bit(data, DataMode::Single)
}

pub struct St7701<'a, S> {
    spi: S,
    rst: Output<'a>,
}

pub struct ManualSpi<'a> {
    pub cs: Output<'a>,
    pub sda: Flex<'a>,
    pub scl: Output<'a>,
}

impl<'a, S> St7701<'a, S> {
    pub fn new(spi: S, rst: Output<'a>) -> Self {
        Self { spi, rst }
    }
}

pub trait SpiProvider {
    type Error;

    fn write_byte(&mut self, is_command: bool, byte: u8) -> Result<(), Self::Error>;

    fn write_command(&mut self, command: u8) -> Result<(), Self::Error> {
        self.while_cs(|s| s.write_byte(true, command))
    }

    fn write_data(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.while_cs(|s| data.iter().try_for_each(|byte| s.write_byte(false, *byte)))
    }

    fn while_cs<F, R>(&mut self, func: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        func(self)
    }
}

impl<Dm: DriverMode> SpiProvider for Spi<'_, Dm> {
    type Error = Error;

    fn write_byte(&mut self, is_command: bool, byte: u8) -> Result<(), Self::Error> {
        self.half_duplex_write(
            DataMode::Single,
            ser(is_command, byte),
            Address::None,
            0,
            &[],
        )
    }

    fn write_command(&mut self, instruction: u8) -> Result<(), Self::Error> {
        self.half_duplex_write(
            DataMode::Single,
            ser(true, instruction),
            Address::None,
            0,
            &[],
        )
    }

    fn write_data(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        for byte in data {
            self.half_duplex_write(DataMode::Single, ser(false, *byte), Address::None, 0, &[])?;
        }

        Ok(())
    }
}

impl SpiProvider for ManualSpi<'_> {
    type Error = Infallible;

    fn while_cs<F, R>(&mut self, func: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.cs.set_low();
        Delay::new().delay_ms(1);
        let result = func(self);
        Delay::new().delay_ms(1);
        self.cs.set_high();
        result
    }

    fn write_byte(&mut self, is_command: bool, byte: u8) -> Result<(), Self::Error> {
        self.sda.set_as_output();

        let mut data = byte;

        self.scl.set_low();
        // First bit: 0 for command, 1 for parameter
        if is_command {
            self.sda.set_low()
        } else {
            self.sda.set_high()
        }
        self.scl.set_high();

        for _ in 0..u8::BITS {
            Delay::new().delay_ns(100);

            self.scl.set_low();

            if data & MSB_MASK == MSB_MASK {
                self.sda.set_high();
            } else {
                self.sda.set_low();
            }

            self.scl.set_high();

            data <<= 1;
        }

        Delay::new().delay_ns(100);

        self.scl.set_high();

        Ok(())
    }
}

impl<S: SpiProvider> St7701<'_, S> {
    pub fn reset(&mut self, delay: &mut impl DelayNs) {
        self.rst.set_high();
        delay.delay_ms(100);
        self.rst.set_low();
        delay.delay_ms(100);
        self.rst.set_high();
        delay.delay_ms(100);
    }

    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), S::Error> {
        self.reset(delay);

        self.spi.write_command(0xFF)?;
        self.spi.write_data(&[0x77, 0x01, 0x00, 0x00, 0x10])?;

        self.spi.write_command(0xC0)?;
        self.spi.write_data(&[0x3B, 0x00])?;
        self.spi.write_command(0xC1)?;
        self.spi.write_data(&[0x0B, 0x02])?; // VBP
        self.spi.write_command(0xC2)?;
        self.spi.write_data(&[0x00, 0x02])?;

        self.spi.write_command(0xCC)?;
        self.spi.write_data(&[0x10])?;
        self.spi.write_command(0xCD)?;
        self.spi.write_data(&[0x08])?;

        self.spi.write_command(0xB0)?; // Positive Voltage Gamma Control
        self.spi.write_data(&[
            0x02, 0x13, 0x1B, 0x0D, 0x10, 0x05, 0x08, 0x07, 0x07, 0x24, 0x04, 0x11, 0x0E, 0x2C,
            0x33, 0x1D,
        ])?;

        self.spi.write_command(0xB1)?; // Negative Voltage Gamma Control
        self.spi.write_data(&[
            0x05, 0x13, 0x1B, 0x0D, 0x11, 0x05, 0x08, 0x07, 0x07, 0x24, 0x04, 0x11, 0x0E, 0x2C,
            0x33, 0x1D,
        ])?;

        self.spi.write_command(0xFF)?;
        self.spi.write_data(&[0x77, 0x01, 0x00, 0x00, 0x11])?;

        self.spi.write_command(0xB0)?;
        self.spi.write_data(&[0x5d])?; // 5d
        self.spi.write_command(0xB1)?;
        self.spi.write_data(&[0x43])?; // VCOM amplitude setting
        self.spi.write_command(0xB2)?;
        self.spi.write_data(&[0x81])?; // VGH Voltage setting, 12V
        self.spi.write_command(0xB3)?;
        self.spi.write_data(&[0x80])?;

        self.spi.write_command(0xB5)?;
        self.spi.write_data(&[0x43])?; // VGL Voltage setting, -8.3V

        self.spi.write_command(0xB7)?;
        self.spi.write_data(&[0x85])?;
        self.spi.write_command(0xB8)?;
        self.spi.write_data(&[0x20])?;

        self.spi.write_command(0xC1)?;
        self.spi.write_data(&[0x78])?;
        self.spi.write_command(0xC2)?;
        self.spi.write_data(&[0x78])?;

        self.spi.write_command(0xD0)?;
        self.spi.write_data(&[0x88])?;

        self.spi.write_command(0xE0)?;
        self.spi.write_data(&[0x00, 0x00, 0x02])?;

        self.spi.write_command(0xE1)?;
        self.spi.write_data(&[
            0x03, 0xA0, 0x00, 0x00, 0x04, 0xA0, 0x00, 0x00, 0x00, 0x20, 0x20,
        ])?;

        self.spi.write_command(0xE2)?;
        self.spi.write_data(&[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])?;

        self.spi.write_command(0xE3)?;
        self.spi.write_data(&[0x00, 0x00, 0x11, 0x00])?;

        self.spi.write_command(0xE4)?;
        self.spi.write_data(&[0x22, 0x00])?;

        self.spi.write_command(0xE5)?;
        self.spi.write_data(&[
            0x05, 0xEC, 0xA0, 0xA0, 0x07, 0xEE, 0xA0, 0xA0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ])?;

        self.spi.write_command(0xE6)?;
        self.spi.write_data(&[0x00, 0x00, 0x11, 0x00])?;

        self.spi.write_command(0xE7)?;
        self.spi.write_data(&[0x22, 0x00])?;

        self.spi.write_command(0xE8)?;
        self.spi.write_data(&[
            0x06, 0xED, 0xA0, 0xA0, 0x08, 0xEF, 0xA0, 0xA0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ])?;

        self.spi.write_command(0xEB)?;
        self.spi
            .write_data(&[0x00, 0x00, 0x40, 0x40, 0x00, 0x00, 0x00])?;

        self.spi.write_command(0xED)?;
        self.spi.write_data(&[
            0xFF, 0xFF, 0xFF, 0xBA, 0x0A, 0xBF, 0x45, 0xFF, 0xFF, 0x54, 0xFB, 0xA0, 0xAB, 0xFF,
            0xFF, 0xFF,
        ])?;

        self.spi.write_command(0xEF)?;
        self.spi.write_data(&[0x10, 0x0D, 0x04, 0x08, 0x3F, 0x1F])?;

        self.spi.write_command(0xFF)?;
        self.spi.write_data(&[0x77, 0x01, 0x00, 0x00, 0x13])?;

        self.spi.write_command(0xEF)?;
        self.spi.write_data(&[0x08])?;

        self.spi.write_command(0xFF)?;
        self.spi.write_data(&[0x77, 0x01, 0x00, 0x00, 0x00])?;

        self.spi.write_command(0x36)?;
        self.spi.write_data(&[0x08])?;
        self.spi.write_command(0x3A)?;
        self.spi.write_data(&[0x60])?; // 0x70 RGB888, 0x60 RGB666, 0x50 RGB565

        self.spi.write_command(0x11)?; // Sleep Out

        Delay::new().delay_ms(100);

        self.spi.write_command(0x29)?; // Display On

        Delay::new().delay_ms(50);

        Ok(())
    }
}
