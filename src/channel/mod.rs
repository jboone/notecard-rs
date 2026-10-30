use defmt::Format;

pub mod asynch;
pub mod blocking;

pub trait BaudRate {
    fn set_baudrate(&mut self, rate: u32) -> Result<(), Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Format)]
#[non_exhaustive]
pub enum Error {
    Write,
    Read,
    BaudRate,
    InvalidResponse,
    InvalidArgument,
    Handshake,
    Timeout,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Read => f.write_str("Read"),
            Self::Write => f.write_str("Write"),
            Self::BaudRate => f.write_str("BaudRate"),
            Self::InvalidResponse => f.write_str("InvalidResponse"),
            Self::InvalidArgument => f.write_str("InvalidArgument"),
            Self::Handshake => f.write_str("Handshake"),
            Self::Timeout => f.write_str("Timeout"),
        }
    }
}

impl core::error::Error for Error {}
