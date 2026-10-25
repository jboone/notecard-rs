pub mod asynch;
pub mod blocking;

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
