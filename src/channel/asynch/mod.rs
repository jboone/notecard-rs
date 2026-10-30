pub mod uart;

#[allow(async_fn_in_trait)]
pub trait Channel: embedded_io_async::Read + embedded_io_async::ReadReady + embedded_io_async::Write {
    async fn reset(&mut self) -> Result<bool, Self::Error>;
    async fn ping(&mut self) -> Result<bool, Self::Error>;
}

#[allow(async_fn_in_trait)]
pub trait BaudRateNegotiate {
    async fn negotiate_baudrate(&mut self, default_rate: u32, preferred_rate: u32) -> Result<(), super::Error>;
}
