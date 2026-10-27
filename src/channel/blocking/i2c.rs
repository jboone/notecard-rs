use core::cmp::min;

use defmt::{debug, error, trace, warn};
use embassy_time::{Duration, Instant};
use embedded_hal::{delay::DelayNs, i2c::{self, Operation}};
use embedded_io::{Error, ErrorKind, ErrorType, Read, ReadReady, Write};

use crate::{NotecardConfig, channel::{Error as ChannelError, blocking::Channel}};

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

const ALLOC_CHUNK: usize = 128;

const CHAR_LF: u8 = b'\n';
const CHAR_CR: u8 = b'\r';

const NOTE_I2C_HEADER_SIZE: usize = 2;
const NOTE_I2C_MTU_MAX: usize = u8::MAX as usize - NOTE_I2C_HEADER_SIZE;

const CARD_RESET_DRAIN_MS: u64 = 500;
const CARD_RESET_SYNC_RETRIES: usize = 10;

const CARD_REQUEST_I2C_SEGMENT_DELAY_MS: u32 = 250;
const CARD_REQUEST_I2C_CHUNK_DELAY_MS: u32 = 20;
const CARD_REQUEST_I2C_NACK_WAIT_MS: u32 = 1_000;

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

    const fn i2c_max(&self) -> usize {
        NOTE_I2C_MTU_MAX
    }

    fn delay_io(&mut self) {
        self.delay.delay_ms(6)
    }

    fn i2c_transmit(&mut self, buffer: &[u8]) -> Result<(), ChannelError> {
        // trace!("i2c: transmit: buffer.len() = {}", buffer.len());

        let payload_size = buffer.len();
        if payload_size > NOTE_I2C_MTU_MAX {
            return Err(ChannelError::InvalidArgument);
        }

        let tx_header= [payload_size as u8];
        trace!("i2c: transmit: {=[u8; 1]:02x} {=[u8]:02x}", tx_header, buffer);
        self.i2c.transaction(self.addr, &mut [
            Operation::Write(&tx_header),
            Operation::Write(buffer),
        ]).map_err(|_| ChannelError::Write)
    }

    fn i2c_receive_inner(&mut self, buffer: &mut [u8]) -> Result<usize, ChannelError> {
        // trace!("i2c: receive_inner: buffer.len() = {}", buffer.len());

        let requested_byte_count = buffer.len();
        if requested_byte_count > NOTE_I2C_MTU_MAX {
            error!("i2c: receive_inner: {} > {}", requested_byte_count, NOTE_I2C_MTU_MAX);
            return Err(ChannelError::InvalidArgument);
        }

        let tx_header= [0x00, requested_byte_count as u8];
        trace!("i2c: receive_inner: transmit: {=[u8; 2]:02x}", tx_header);
        self.i2c.write(self.addr, &tx_header).map_err(|_| ChannelError::Write)?;

        self.delay.delay_ms(2);

        let mut rx_header = [0_u8; 2];
        self.i2c.transaction(self.addr, &mut [
            Operation::Read(&mut rx_header),
            Operation::Read(buffer),
        ]).map_err(|_| ChannelError::Read)?;

        trace!("i2c: receive_inner: receive: {=[u8; 2]:02x} {=[u8]:02x}", &rx_header, buffer);
        let available = rx_header[0] as usize;
        let received_byte_count = rx_header[1] as usize;

        if available > NOTE_I2C_MTU_MAX {
            return Err(ChannelError::InvalidResponse);
        } else if received_byte_count != requested_byte_count {
            return Err(ChannelError::InvalidResponse);
        } else {
            if available > 0 {
                trace!("i2c: receive_inner: available = {}", available);
            }
            Ok(available)
        }
    }

    fn i2c_receive(&mut self, buffer: &mut [u8]) -> Result<usize, ChannelError> {
        const RETRY_COUNT: usize = 3;

        for attempt in 0..RETRY_COUNT {
            let last = (attempt + 1) == RETRY_COUNT;
            match self.i2c_receive_inner(buffer) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    trace!("i2c: receive, error {}", e);
                    self.delay.delay_ms(1_000);
                    if last {
                        return Err(e);
                    }
                }
            }
        }

        unreachable!();
    }

    fn data_query(&mut self) -> Result<DataQueryResult, ChannelError> {
        let write_header = [0_u8; 2];
        let mut read_header = [0_u8; 2];

        self.i2c
            .write_read(self.addr, &write_header, &mut read_header)
            .map_err(|_| ChannelError::Write)?;

        let result: DataQueryResult = read_header.into();
        trace!("data_query: available_to_send={} sent_in_this_response={}", result.available_to_send, result.sent_in_this_response);

        if result.sent_in_this_response != 0 {
            error!("data_query sent_in_this_response {} != 0", result.sent_in_this_response);
            Err(ChannelError::InvalidResponse)
        } else {
            Ok(result)
        }
    }
}

impl Error for ChannelError {
    fn kind(&self) -> ErrorKind {
        match *self {
            ChannelError::Read => ErrorKind::ConnectionAborted,
            ChannelError::Write => ErrorKind::ConnectionAborted,
            ChannelError::InvalidResponse => ErrorKind::InvalidData,
            ChannelError::InvalidArgument => ErrorKind::InvalidInput,
            ChannelError::Timeout => ErrorKind::TimedOut,
        }
    }
}

impl<T, D> ErrorType for I2cChannel<T, D> {
    type Error = ChannelError;
}

impl<T, D> Read for I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    fn read(&mut self, mut buf: &mut [u8]) -> Result<usize, Self::Error> {
        let dqr = self.data_query()?;
        let mut available = dqr.available_to_send;

        let mut bytes_read = 0;
        while buf.len() > 0 && available > 0 {
            let payload_size = min(min(NOTE_I2C_MTU_MAX, buf.len()), available);

            let (read_payload, unused_buf) = buf.split_at_mut(payload_size);

            available = self.i2c_receive(read_payload)?;

            bytes_read += payload_size;
            trace!("read: bytes_read={}", bytes_read);

            buf = unused_buf;
        }

        trace!("read: total bytes_read={}", bytes_read);
        Ok(bytes_read)
    }
}

impl<T, D> ReadReady for I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(self.data_query()?.available_to_send > 0)
    }
}

impl<T, D> Write for I2cChannel<T, D>
where
    T: i2c::I2c,
    D: DelayNs,
{
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let payload_size = min(NOTE_I2C_MTU_MAX, buf.len());
        let payload = &buf[..payload_size];

        trace!("write: sending payload {}, writing {}", payload_size, 1 + payload_size);

        let now = Instant::now();
        let ms_until_write_ok = if now >= self.next_write_ok_at { 0 } else { (self.next_write_ok_at - now).as_millis() as u32 };
        if ms_until_write_ok > 0 {
            trace!("write: waiting for {} ms", ms_until_write_ok);
            self.delay.delay_ms(ms_until_write_ok);
        }

        self.i2c_transmit(payload)?;

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
    /// https://github.com/blues/note-c/blob/master/n_i2c.c#L195
    fn reset(&mut self) -> Result<bool, Self::Error> {
        trace!("i2c: reset");

        let mut notecard_ready = false;

        self.delay.delay_ms(CARD_REQUEST_I2C_SEGMENT_DELAY_MS);

        // TODO: I2C reset.
        trace!("i2c: pretending to reset bus");

        self.delay_io();

        for retry in 0..CARD_RESET_SYNC_RETRIES {
            trace!("i2c: notecard reset sync attempt {}", retry);

            trace!("i2c: writing single LF");
            if let Err(e) = self.i2c_transmit(&[CHAR_LF]) {
                error!("i2c: write of LF failed, swallowing error {}", e);
                self.delay.delay_ms(CARD_REQUEST_I2C_NACK_WAIT_MS);
                notecard_ready = false;
                continue;
            }

            self.delay.delay_ms(CARD_REQUEST_I2C_SEGMENT_DELAY_MS);

            let mut chunk_len = 0;
            let mut something_found = false;
            let mut non_control_char_found = false;

            let timeout_duration = Duration::from_millis(CARD_RESET_DRAIN_MS);
            let mut timeout = Instant::now() + timeout_duration;

            while Instant::now() < timeout {

                let mut buffer = [0_u8; ALLOC_CHUNK];
                chunk_len = min(chunk_len, buffer.len());
                chunk_len = min(chunk_len, self.i2c_max());

                let available = match self.i2c_receive(&mut buffer[..chunk_len]) {
                    Ok(v) => v,
                    Err(e) => {
                        error!("i2c: error encountered during I2C receive hook execution: {}", e);
                        self.delay.delay_ms(CARD_REQUEST_I2C_SEGMENT_DELAY_MS);
                        continue;
                    }
                };

                if chunk_len != 0 {
                    something_found = true;
                    for &ch in &buffer[..chunk_len] {
                        if ch != CHAR_LF && ch != CHAR_CR {
                            non_control_char_found = true;
                            timeout = Instant::now() + timeout_duration;
                        }
                    }
                }

                chunk_len = min(available, 0xffff);

                self.delay.delay_ms(CARD_REQUEST_I2C_CHUNK_DELAY_MS);
            }

            if !something_found || non_control_char_found {
                notecard_ready = false;

                if something_found {
                    warn!("i2c: unrecognized data from notecard");
                } else {
                    error!("i2c: notecard not responding");

                    // TODO: I2C reset.
                    trace!("i2c: pretending to reset bus");

                    self.delay_io();
                }
            } else {
                notecard_ready = true;
                break;
            }

            debug!("i2c: retrying interface reset...");
        }

        Ok(notecard_ready)
    }

    fn ping(&mut self) -> bool {
        self.i2c.read(self.addr, &mut []).is_ok()
    }
}
