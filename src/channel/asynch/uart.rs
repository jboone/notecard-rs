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
    async fn ping(&mut self) -> bool {
        // There isn't really a "ping" function over UART, that I'm aware of.

        // self.uart_tx.write_all(b"\n").await.unwrap();
        // self.uart_tx.flush().await.unwrap();

        // let mut buffer = [0_u8; 256];
        // while let Ok(true) = self.uart_rx.read_ready() {
        //     match self.uart_rx.read(&mut buffer).await {
        //         Ok(count) => {
        //             let bytes = &buffer[..count];
        //         },
        //         Err(e) => {

        //         },
        //     }
        // }
        true
    }

    async fn reset(&mut self) -> Result<bool, Self::Error> {
        // TODO: Just implement a `flush()`?
        Ok(true)
    }
}
