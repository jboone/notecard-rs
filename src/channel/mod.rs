pub mod i2c;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    Write,
    Read,
    InvalidResponse,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Read => f.write_str("Read"),
            Self::Write => f.write_str("Write"),
            Self::InvalidResponse => f.write_str("InvalidResponse"),
        }
    }
}

impl core::error::Error for Error {}

pub trait ChannelBlocking: embedded_io::Read + embedded_io::ReadReady + embedded_io::Write {
    fn ping(&mut self) -> bool;
}

#[allow(async_fn_in_trait)]
pub trait ChannelAsync: embedded_io_async::Read + embedded_io_async::ReadReady + embedded_io_async::Write {
    async fn ping(&mut self) -> bool;
}
