use core::cmp::min;

use defmt::{error, trace};
use embassy_time::{Duration, Instant};
use embedded_hal::{delay::DelayNs, i2c::{self, Operation}};

use crate::{NotecardConfig, channel::{Error, blocking::Channel}};

struct DataQueryResult {
    available_to_send: usize,
    sent_in_this_response: usize,
}

impl From<[u8; 2]> for DataQueryResult {
    fn from(value: [u8; 2]) -> Self {
        Self {
            available_to_send: value[0] as usize,
            sent_in_this_response: value[1] as usize,
        }
    }
}

pub struct I2cChannel<T, D> {
    i2c: T,
    addr: i2c::SevenBitAddress,
    config: NotecardConfig,
    delay: D,
    next_write_ok_at: Instant,
}

const NOTE_I2C_PAYLOAD_MAX: usize = 253;

impl<T, D> I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    pub fn new(i2c: T, delay: D) -> Self {
        Self {
            i2c,
            addr: 0x17,
            config: NotecardConfig::default(),
            delay,
            next_write_ok_at: Instant::now(),
        }
    }

    fn data_query(&mut self) -> Result<DataQueryResult, Error> {
        let write_header = [0_u8; 2];
        let mut read_header = [0_u8; 2];

        self.i2c
            .write_read(self.addr, &write_header, &mut read_header)
            .map_err(|_| Error::Write)?;

        let result: DataQueryResult = read_header.into();
        trace!("data_query: available_to_send={} sent_in_this_response={}", result.available_to_send, result.sent_in_this_response);

        if result.sent_in_this_response != 0 {
            error!("data_query sent_in_this_response {} != 0", result.sent_in_this_response);
            Err(Error::InvalidResponse)
        } else {
            Ok(result)
        }
    }
}

impl embedded_io::Error for crate::channel::Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        match *self {
            Error::Read => embedded_io::ErrorKind::ConnectionAborted,
            Error::Write => embedded_io::ErrorKind::ConnectionAborted,
            Error::InvalidResponse => embedded_io::ErrorKind::InvalidData,
        }
    }
}

impl<T, D> embedded_io::ErrorType for I2cChannel<T, D> {
    type Error = crate::channel::Error;
}

impl<T, D> embedded_io::Read for I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    fn read(&mut self, mut buf: &mut [u8]) -> Result<usize, Self::Error> {
        let dqr = self.data_query()?;
        let mut available = dqr.available_to_send;

        let mut bytes_read = 0;
        while buf.len() > 0 && available > 0 {
            let payload_size = min(min(NOTE_I2C_PAYLOAD_MAX, buf.len()), available);

            let write_header = [0x00, payload_size.try_into().unwrap()];
            let mut read_header = [0_u8; 2];
            let (read_payload, unused_buf) = buf.split_at_mut(payload_size);

            trace!("read: requesting payload {}, reading {}", payload_size, 2 + payload_size);
            self.i2c.transaction(self.addr, &mut [
                Operation::Write(&write_header),
                Operation::Read(&mut read_header),
                Operation::Read(read_payload),
            ]).map_err(|_| Error::Read)?;

            let dqr: DataQueryResult = read_header.into();
            available = dqr.available_to_send;
            trace!("read: available_to_send={} sent_in_this_response={}", available, dqr.sent_in_this_response);

            // Make sure the response says we were given the number of bytes we asked for.
            // TODO: I'm unsure if this assertion would ever be false, assuming the Notecard
            // firmware isn't buggy, or the I2C bus burped somehow.
            assert_eq!(dqr.sent_in_this_response, payload_size);

            bytes_read += dqr.sent_in_this_response;
            trace!("read: bytes_read={}", bytes_read);

            buf = unused_buf;
        }

        trace!("read: total bytes_read={}", bytes_read);
        Ok(bytes_read)
    }
}

impl<T, D> embedded_io::ReadReady for I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(self.data_query()?.available_to_send > 0)
    }
}

impl<T, D> embedded_io::Write for I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let payload_size = min(NOTE_I2C_PAYLOAD_MAX, buf.len());
        let header = [payload_size.try_into().unwrap()];
        let payload = &buf[..payload_size];

        trace!("write: sending payload {}, writing {}", payload_size, 1 + payload_size);

        let now = Instant::now();
        let ms_until_write_ok = if now >= self.next_write_ok_at { 0 } else { (self.next_write_ok_at - now).as_millis() as u32 };
        if ms_until_write_ok > 0 {
            trace!("write: waiting for {} ms", ms_until_write_ok);
            self.delay.delay_ms(ms_until_write_ok);
        }

        self.i2c.transaction(self.addr, &mut [
            Operation::Write(&header),
            Operation::Write(payload),
        ]).map_err(|_| Error::Write)?;

        self.next_write_ok_at = Instant::now() + Duration::from_millis(self.config.segment_delay.into());

        Ok(payload_size)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<T, D> Channel for I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    fn ping(&mut self) -> bool {
        self.i2c.read(self.addr, &mut []).is_ok()
    }
}
