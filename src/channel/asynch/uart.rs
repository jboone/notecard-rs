use core::fmt::Write as _;

use defmt::{debug, error, info, trace, warn};
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::{ErrorType, Read, ReadReady, Write};

use crate::channel::{BaudRate, Error};
use super::Channel;

pub struct UartChannel<U, D> {
    uart: U,
    delay: D,
}

impl<U, D> UartChannel<U, D>
where
    U: Write + Read + ReadReady,
    D: DelayNs,
{
    pub fn new_async(uart: U, delay: D) -> Self {
        Self {
            uart,
            delay,
        }
    }
}

impl<U, D> ErrorType for UartChannel<U, D> {
    type Error = crate::channel::Error;
}

impl<U, D> Read for UartChannel<U, D>
where
    U: Read
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let count = self.uart.read(buf).await.map_err(|_| Error::Read)?;
        trace!("uart: read {:a}", &buf[..count]);
        Ok(count)
    }
}

impl<U, D> ReadReady for UartChannel<U, D>
where
    U: ReadReady
{
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        self.uart.read_ready().map_err(|_| Error::Read)
    }
}

impl<U, D> Write for UartChannel<U, D>
where
    U: Write
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        trace!("uart: write {:a}", buf);
        self.uart.write(buf).await.map_err(|_| Error::Write)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.uart.flush().await.map_err(|_| Error::Write)
    }
}

const CARD_RESET_DRAIN_MS: u32 = 500;

impl<U, D> Channel for UartChannel<U, D>
where
    U: Write + Read + ReadReady,
    D: DelayNs,
{
    async fn ping(&mut self) -> Result<bool, Self::Error> {
        let mut buffer = [0_u8; 256];

        trace!("notecard: sending single LF");
        self.write_all(b"\n").await.map_err(|_| Error::Write)?;

        trace!("notecard: waiting for handshake");
        let mut waited = 0;
        let mut something_found = false;
        let mut non_control_char_found = false;

        while waited < CARD_RESET_DRAIN_MS {
            while let Ok(true) = self.read_ready() {
                match self.read(&mut buffer).await {
                    Ok(count) => {
                        let read = &buffer[..count];
                        trace!("notecard: read {:a}", read);
                        something_found = true;
                        for &c in read {
                            match c {
                                b'\n' | b'\r' => {
                                    trace!("notecard: found CR or LF");
                                },
                                _ => {
                                    non_control_char_found = true;
                                    waited = 0;
                                },
                            }
                        }
                    },
                    Err(e) => {
                        trace!("notecard: read error");
                    },
                }
            }

            self.delay.delay_ms(1).await;
            waited += 1;
        }

        if something_found && !non_control_char_found {
            trace!("notecard: ready");
            return Ok(true);
        }

        if something_found {
            error!("unrecognized data from notecard");
        } else {
            error!("notecard not responding");
        }

        Ok(false)
    }

    async fn reset(&mut self) -> Result<bool, Self::Error> {
        for _ in 0..10 {
            if self.ping().await? {
                return Ok(true);
            }

            self.delay.delay_ms(CARD_RESET_DRAIN_MS).await;

            // TODO: "Reset" serial port.
            warn!("TODO: reset serial port");

            debug!("retrying Serial interface reset.")
        }

        Ok(false)
    }
}

impl<U, D> BaudRate for UartChannel<U, D>
where
    U: Write + Read + ReadReady + BaudRate,
{
    fn set_baudrate(&mut self, rate: u32) -> Result<(), Error> {
        self.uart.set_baudrate(rate)?;
        Ok(())
    }
}

impl<U, D> super::BaudRateNegotiate for UartChannel<U, D>
where
    U: Write + Read + ReadReady + BaudRate,
    D: DelayNs,
{
    /// Function assumes UART is already configured for preferred rate.
    async fn negotiate_baudrate(&mut self, default_rate: u32, preferred_rate: u32) -> Result<(), Error> {
        info!("notecard: AUX serial: initializing interface");

        if !self.reset().await? {
            warn!("notecard: AUX serial: failed at preferred rate");
            self.set_baudrate(default_rate).unwrap();

            if !self.reset().await? {
                error!("notecard: AUX serial: failed at default rate");
                return Err(Error::Handshake)
            } else {
                warn!("notecard: AUX serial: communicating at default rate, attempting upgrade to preferred rate");
                let mut request = heapless::Vec::<u8, 128>::new();
                request.write_fmt(format_args!("{{\"req\":\"card.aux.serial\",\"rate\":{}}}\n", preferred_rate)).unwrap();
                self.write_all(&request).await.map_err(|_| Error::Write)?;

                self.set_baudrate(preferred_rate).unwrap();

                if !self.reset().await? {
                    error!("notecard: AUX serial: failed to shift from default to preferred rate");
                return Err(Error::Handshake)
                }
            }
        } else {
            info!("notecard: AUX serial: communicating at preferred rate");
        }

        Ok(())
    }
}
