use defmt::{debug, error, info, trace, warn};
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::{ErrorType, Read, ReadReady, Write};

use crate::channel::Error;
use super::Channel;

pub struct UartChannel<T, U, D> {
    uart_tx: T,
    uart_rx: U,
    delay: D,
}

impl<T, U, D> UartChannel<T, U, D>
where
    T: Write,
    U: Read + ReadReady,
    D: DelayNs,
{
    pub fn new_async(uart_tx: T, uart_rx: U, delay: D) -> Self {
        Self {
            uart_tx,
            uart_rx,
            delay,
        }
    }
}

impl<T, U, D> ErrorType for UartChannel<T, U, D> {
    type Error = crate::channel::Error;
}

impl<T, U, D> Read for UartChannel<T, U, D>
where
    U: Read
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let count = self.uart_rx.read(buf).await.map_err(|_| Error::Read)?;
        trace!("uart: read \"{:a}\"", &buf[..count]);
        Ok(count)
    }
}

impl<T, U, D> ReadReady for UartChannel<T, U, D>
where
    U: ReadReady
{
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        self.uart_rx.read_ready().map_err(|_| Error::Read)
    }
}

impl<T, U, D> Write for UartChannel<T, U, D>
where
    T: Write
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        trace!("uart: write \"{:a}\"", buf);
        self.uart_tx.write(buf).await.map_err(|_| Error::Write)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.uart_tx.flush().await.map_err(|_| Error::Write)
    }
}

const CARD_RESET_DRAIN_MS: u32 = 500;

impl<T, U, D> Channel for UartChannel<T, U, D>
where
    T: Write,
    U: Read + ReadReady,
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
