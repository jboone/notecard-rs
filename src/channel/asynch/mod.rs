pub mod uart;

#[allow(async_fn_in_trait)]
pub trait Channel: embedded_io_async::Read + embedded_io_async::ReadReady + embedded_io_async::Write {
    async fn reset(&mut self) -> Result<bool, Self::Error>;
    async fn ping(&mut self) -> bool;
}
