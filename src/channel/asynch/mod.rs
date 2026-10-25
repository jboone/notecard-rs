
#[allow(async_fn_in_trait)]
pub trait Channel: embedded_io_async::Read + embedded_io_async::ReadReady + embedded_io_async::Write {
    async fn ping(&mut self) -> bool;
}
