pub mod i2c;

pub trait Channel: embedded_io::Read + embedded_io::ReadReady + embedded_io::Write {
    fn reset(&mut self) -> Result<bool, Self::Error>;
    fn ping(&mut self) -> bool;
}
