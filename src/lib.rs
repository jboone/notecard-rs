//! Protocol for transmitting: <https://dev.blues.io/notecard/notecard-guides/serial-over-i2c-protocol/>
//! API: <https://dev.blues.io/reference/notecard-api/introduction/>
//!
#![feature(type_changing_struct_update)]
#![cfg_attr(not(test), no_std)]

use core::convert::Infallible;
use core::marker::PhantomData;

#[allow(unused_imports)]
use defmt::{debug, error, info, trace, warn};

use embedded_hal::delay::DelayNs;
use heapless::{String, Vec};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::channel::ChannelBlocking;

pub mod card;
pub mod channel;
pub mod dfu;
pub mod env;
pub mod hub;
pub mod note;
pub mod web;
pub mod ntn;

/// Delay between polling for new response.
const RESPONSE_DELAY: u32 = 25;

/// The size of the shared request and receive buffer. Requests and responses may not serialize to
/// any greater value than this.
pub const DEFAULT_BUF_SIZE: usize = 18 * 1024;

#[derive(Debug, defmt::Format)]
pub struct NotecardConfig {
    /// Timeout while waiting for response (ms).
    pub response_timeout: u32,

    /// Delay between chunks when transmitting (ms).
    ///
    /// See note on `segment_delay`.
    ///
    /// > `note-c`: https://github.com/blues/note-c/blob/master/n_lib.h#L52
    /// > Original: 20 ms
    pub chunk_delay: u32,

    /// Delay between segments when transmitting (ms).
    ///
    /// > These delay may be almost eliminated for Notecard firmware version 3.4 (and presumably
    /// > above).
    ///
    /// > `note-c`: https://github.com/blues/note-c/blob/master/n_lib.h#L46
    /// > Original: 250 ms.
    pub segment_delay: u32,
}

impl Default for NotecardConfig {
    fn default() -> Self {
        NotecardConfig {
            response_timeout: 5000,
            chunk_delay: 20,
            segment_delay: 250,
        }
    }
}

#[derive(Debug, defmt::Format)]
pub enum NoteState {
    /// Perform handshake with Notecard.
    Handshake,

    /// Ready to make request.
    Request,

    /// Waiting for response to become ready, value is tries made.
    Poll(usize),

    /// Reading response, value is remaining bytes.
    Response,

    /// Full response has been read into `buf`.
    ResponseReady,
}

#[derive(Debug, defmt::Format, Clone)]
pub enum NoteError {
    I2cWriteError,

    I2cReadError,

    DeserError(String<256>),

    SerError,

    /// Request does not end with '\n'.
    InvalidRequest,

    RemainingData,

    TimeOut,

    BufOverflow,

    /// Method called when notecarrier is in invalid state.
    WrongState,

    /// Notecard firmware is being updated.
    DFUInProgress,

    /// Notecard is in NTN mode and regular packages can't be added.
    NonPortNoteInPackageMode,

    /// Notecard filesystem full
    FileStorageFull(String<256>),

    /// Error Adding Note
    ErrorAddingNote(String<256>),

    NotecardErr(String<256>),
}

impl NoteError {
    pub fn new_desererror(msg: &[u8]) -> NoteError {
        let msg = core::str::from_utf8(msg).unwrap_or("[invalid utf-8]");
        let mut s = String::new();
        s.push_str(msg).ok();
        NoteError::DeserError(s)
    }

    pub fn string_err(_e: Infallible) -> NoteError {
        NoteError::BufOverflow
    }
}

pub(crate) fn str_string<const N: usize>(
    a: Option<&str>,
) -> Result<Option<heapless::String<N>>, NoteError> {
    a.map(heapless::String::try_from)
        .transpose()
        .map_err(NoteError::string_err)
}

#[derive(Deserialize, Debug, defmt::Format)]
pub struct NotecardError {
    err: String<256>,
}

impl From<NotecardError> for NoteError {
    fn from(n: NotecardError) -> NoteError {
        if n.err.contains("{dfu-in-progress}") {
            NoteError::DFUInProgress
        } else if n.err.contains("adding notes to a non-uplinked port is not allowed") {
            NoteError::NonPortNoteInPackageMode
        } else if n.err.contains("{file-storage-full}") {
            NoteError::FileStorageFull(n.err)
        } else if n.err.contains("error adding note") {
            NoteError::ErrorAddingNote(n.err)
        } else {
            NoteError::NotecardErr(n.err)
        }
    }
}

/// The driver for the Notecard. Must be intialized before making any requests.
pub struct Notecard<
    IOM: ChannelBlocking,
    const BUF_SIZE: usize = DEFAULT_BUF_SIZE,
> {
    channel: IOM,
    state: NoteState,

    /// The receive buffer. Must be large enough to hold the largest response that will be received.
    buf: Vec<u8, BUF_SIZE>,

    response_timeout: u32,
}

pub struct SuspendState<const BUF_SIZE: usize> {
    state: NoteState,
    buf: Vec<u8, BUF_SIZE>,
    response_timeout: u32,
}

impl<IOM: ChannelBlocking, const BUF_SIZE: usize>
    Notecard<IOM, BUF_SIZE>
{
    pub fn new(channel: IOM) -> Notecard<IOM, BUF_SIZE> {
        Self::new_with_config(channel, NotecardConfig::default())
    }

    pub fn new_with_config(channel: IOM, c: NotecardConfig) -> Notecard<IOM, BUF_SIZE> {
        Notecard {
            channel,
            state: NoteState::Handshake,
            buf: Vec::new(),

            response_timeout: c.response_timeout,
        }
    }

    /// Resize the internal buffer, consuming the existing, and returning a new Notecard
    /// instance.
    pub fn resize_buf<const B: usize>(self) -> Result<Notecard<IOM, B>, NoteError> {
        if B < self.buf.len() {
            Err(NoteError::BufOverflow)
        } else {
            Ok(Notecard {
                buf: Vec::<_, B>::from_slice(&self.buf).unwrap(),
                ..self
            })
        }
    }

    /// Free the IOM device and return the driver state so that it can be quickly resumed. It is
    /// not safe to change the state of the Notecard in the meantime, or create a second driver
    /// without using this state.
    pub fn suspend(self) -> (IOM, SuspendState<BUF_SIZE>) {
        (
            self.channel,
            SuspendState {
                state: self.state,
                buf: self.buf,
                response_timeout: self.response_timeout,
            },
        )
    }

    /// Resume a previously [`suspend`]ed Notecard driver.
    pub fn resume(channel: IOM, state: SuspendState<BUF_SIZE>) -> Notecard<IOM, BUF_SIZE> {
        Notecard {
            channel,
            state: state.state,
            buf: state.buf,
            response_timeout: state.response_timeout,
        }
    }

    /// Initialize the notecard driver by performing handshake with notecard.
    pub fn initialize(&mut self, delay: &mut impl DelayNs) -> Result<(), NoteError> {
        info!("note: initializing.");
        self.reset(delay)
    }

    /// Check if notecarrier is connected and responding.
    ///
    /// > This is allowed no matter the state.
    pub fn ping(&mut self) -> bool {
        self.channel.ping()
    }

    /// Query the notecard for available bytes.
    pub fn data_query(&mut self) -> Result<bool, NoteError> {
        trace!("note: data_query: {:?}", self.state);
        if !matches!(self.state, NoteState::Response) {
            let read_ready = self.channel.read_ready().map_err(|_| NoteError::I2cReadError)?;

            if read_ready {
                self.buf.clear();
                self.state = NoteState::Response;
            }

            trace!("read_ready = {}", read_ready);

            Ok(read_ready)
        } else {
            error!("note: data_query called while reading response.");
            Err(NoteError::WrongState)
        }
    }

    /// Read until empty.
    fn read(&mut self) -> Result<bool, NoteError> {
        if let NoteState::Response = self.state {
            let mut bytes = [0_u8; 256];
            let bytes_read = self.channel.read(&mut bytes).map_err(|_| NoteError::I2cReadError)?;
            self.buf.extend_from_slice(&bytes[..bytes_read]).unwrap(); // XXX: check enough space

            let read_ready = self.channel.read_ready().map_err(|_| NoteError::I2cReadError)?;
            if read_ready {
                self.state = NoteState::Response;
            } else {
                self.state = NoteState::ResponseReady;
            }

            Ok(read_ready)
        } else {
            error!("read: called when not waiting for response");
            Err(NoteError::WrongState)
        }
    }

    /// Take the response from the buffer. Once this function has been called, the state is reset
    /// and it is no longer safe to read the buffer.
    ///
    /// Safety:
    ///
    /// This function returns an immutable reference to the buffer, but new requests require a
    /// mutable reference to `Note`. This is not granted before the immutable reference is
    /// released.
    fn take_response(&mut self) -> Result<&[u8], NoteError> {
        if matches!(self.state, NoteState::ResponseReady) {
            self.state = NoteState::Request;

            Ok(&self.buf)
        } else {
            error!("take response called when response not ready");
            Err(NoteError::WrongState)
        }
    }

    /// Poll for data.
    fn poll(&mut self) -> Result<Option<&[u8]>, NoteError> {
        trace!("note: poll: {:?}", self.state);
        match self.state {
            NoteState::Poll(_) => {
                // 1. Check for available data
                let read_ready = self.data_query()?;
                if read_ready {
                    debug!("response ready");

                    self.poll()
                } else {
                    // sleep and wait for ready.
                    Ok(None)
                }
            }
            NoteState::Response => {
                let read_ready = self.read()?;
                if !read_ready {
                    self.poll()
                } else {
                    // sleep and wait for more data.
                    Ok(None)
                }
            }
            NoteState::ResponseReady => {
                debug!("response read, deserializing.");
                Ok(Some(self.take_response()?))
            }
            _ => {
                error!("poll called when not receiving response");
                Err(NoteError::WrongState)
            }
        }
    }

    /// Read any remaining data from the Notecarrier. This will cancel any waiting responses, and
    /// waiting for a response after this call will time-out.
    unsafe fn consume_response(&mut self, delay: &mut impl DelayNs) -> Result<(), NoteError> {
        warn!("note: trying to consume any left-over response.");
        let mut waited = 0;

        while waited < self.response_timeout {
            if (self.poll()?).is_some() {
                self.buf.clear();
                return Ok(());
            }

            delay.delay_ms(RESPONSE_DELAY);
            waited += RESPONSE_DELAY;
        }

        self.buf.clear();

        error!("response timed out (>= {}).", self.response_timeout);
        Err(NoteError::TimeOut)
    }

    /// Reset notecard driver and state. Any waiting responses will be invalidated
    /// and time-out. However, you won't be able to get a mutable reference without having
    /// dropped the `FutureResponse`.
    pub fn reset(&mut self, delay: &mut impl DelayNs) -> Result<(), NoteError> {
        warn!("resetting: consuming any left-over response and perform a new handshake.");

        self.buf.clear(); // clear in case data_query() is 0.
        self.state = NoteState::Handshake;
        self.handshake(delay)
    }

    fn handshake(&mut self, delay: &mut impl DelayNs) -> Result<(), NoteError> {
        if matches!(self.state, NoteState::Handshake) {
            debug!("note: handshake");
            if self.data_query()? {
                error!("note: handshake: remaining data in queue, consuming..");
                unsafe { self.consume_response(delay)? };
            }

            self.state = NoteState::Request;
        }
        Ok(())
    }

    /// Sends request from buffer.
    fn send_request(&mut self, delay: &mut impl DelayNs) -> Result<(), NoteError> {
        if !matches!(self.state, NoteState::Request) {
            warn!("note: request: wrong-state, resetting before new request.");
            let buf = self.buf.clone();

            self.reset(delay)?; // this clears the buffer - and may use it for other stuff!

            self.buf.clear();
            self.buf
                .resize(buf.len(), 0)
                .map_err(|_| NoteError::BufOverflow)?;
            self.buf.copy_from_slice(&buf);
        }

        if self.buf.last() != Some(&b'\n') {
            return Err(NoteError::InvalidRequest);
        }

        trace!("note: making request: {}", unsafe {
            core::str::from_utf8_unchecked(&self.buf)
        });

        self.channel.write_all(&self.buf).map_err(|_| NoteError::I2cWriteError)?;

        self.state = NoteState::Poll(0);

        Ok(())
    }

    /// Make a raw request. The byte slice must end with `\n`. After making a request a
    /// [FutureResponse] must be created and consumed.
    pub(crate) fn request_raw(
        &mut self,
        delay: &mut impl DelayNs,
        cmd: &[u8],
    ) -> Result<(), NoteError> {
        self.buf.clear();
        self.buf
            .resize(cmd.len(), 0)
            .map_err(|_| NoteError::BufOverflow)?;

        self.buf.copy_from_slice(cmd);

        self.send_request(delay)
    }

    /// Make a request. After making a request a [FutureResponse] must be created and consumed
    /// before making any new requests. This method is usually called through the API methods like
    /// `[card]`.
    pub(crate) fn request<T: Serialize>(
        &mut self,
        delay: &mut impl DelayNs,
        cmd: T,
    ) -> Result<(), NoteError> {
        self.buf.clear();
        self.buf.resize(self.buf.capacity(), 0).unwrap(); // unsafe { set_len } ?

        let sz = serde_json_core::to_slice(&cmd, &mut self.buf).map_err(|_| NoteError::SerError)?;
        self.buf.truncate(sz);

        // Add new-line, this separator tells the Notecard that the request is done.
        self.buf.push(b'\n').map_err(|_| NoteError::SerError)?;
        self.send_request(delay)
    }

    /// [card Requests](https://dev.blues.io/reference/notecard-api/card-requests/)
    pub fn card(&mut self) -> card::Card<'_, IOM, BUF_SIZE> {
        card::Card::from(self)
    }

    pub fn env(&mut self) -> env::Env<'_, IOM, BUF_SIZE> {
        env::Env::from(self)
    }

    /// [note Requests](https://dev.blues.io/reference/notecard-api/note-requests/)
    pub fn note(&mut self) -> note::Note<'_, IOM, BUF_SIZE> {
        note::Note::from(self)
    }

    /// [web Requests](https://dev.blues.io/reference/notecard-api/web-requests/)
    pub fn web(&mut self) -> web::Web<'_, IOM, BUF_SIZE> {
        web::Web::from(self)
    }

    /// [hub Requests](https://dev.blues.io/reference/notecard-api/hub-requests/)
    pub fn hub(&mut self) -> hub::Hub<'_, IOM, BUF_SIZE> {
        hub::Hub::from(self)
    }

    /// [dfu Requests](https://dev.blues.io/api-reference/notecard-api/dfu-requests/)
    pub fn dfu(&mut self) -> dfu::DFU<'_, IOM, BUF_SIZE> {
        dfu::DFU::from(self)
    }

    /// [NtN Requests](https://dev.blues.io/reference/notecard-api/ntn-requests/)
    pub fn ntn(&mut self) -> ntn::NTN<'_, IOM, BUF_SIZE> {
        ntn::NTN::from(self)
    }
}

/// A future response.
///
/// It will not be possible to make any new requests before this has been consumed. If you drop
/// this future before consuming the response the Notecard and driver will be left in inconsistent
/// state. It is not safe to make new requests to the Notecard before the previous response has
/// been read.
#[must_use = "The response must be waited for and consumed, otherwise the notecard is left in an inconsistent state"]
pub struct FutureResponse<
    'a,
    T: DeserializeOwned,
    IOM: ChannelBlocking,
    const BUF_SIZE: usize,
> {
    note: &'a mut Notecard<IOM, BUF_SIZE>,
    _r: PhantomData<T>,
}

impl<
        'a,
        T: DeserializeOwned,
        IOM: ChannelBlocking,
        const BUF_SIZE: usize,
    > FutureResponse<'a, T, IOM, BUF_SIZE>
{
    fn from(note: &'a mut Notecard<IOM, BUF_SIZE>) -> FutureResponse<'a, T, IOM, BUF_SIZE> {
        FutureResponse {
            note,
            _r: PhantomData,
        }
    }

    /// Reads remaining data and returns the deserialized object if it is ready.
    pub fn poll(&mut self) -> Result<Option<T>, NoteError> {
        match self.note.poll()? {
            Some(body) if body.starts_with(br##"{"err":"##) => {
                debug!(
                    "response is error response, parsing error..: {}",
                    core::str::from_utf8(body).unwrap_or("[invalid utf-8]")
                );
                Err(
                    serde_json_core::from_slice::<NotecardError>(body).map_or_else(
                        |_| {
                            error!(
                                "failed to deserialize: {}",
                                core::str::from_utf8(body).unwrap_or("[invalid utf-8]")
                            );
                            NoteError::new_desererror(body)
                        },
                        |(e, _)| NoteError::from(e),
                    ),
                )
            }
            Some(body) => {
                trace!("response is regular, parsing..");
                Ok(Some(
                    serde_json_core::from_slice::<T>(body)
                        .map_err(|_| {
                            error!(
                                "failed to deserialize: {}",
                                core::str::from_utf8(body).unwrap_or("[invalid utf-8]")
                            );
                            NoteError::new_desererror(body)
                        })?
                        .0,
                ))
            }
            None => Ok(None),
        }
    }

    /// Wait for response and return raw bytes. These may change on next response,
    /// so this method is probably not staying as it is.
    pub fn wait_raw(mut self, delay: &mut impl DelayNs) -> Result<&'a [u8], NoteError> {
        let mut waited = 0;

        while waited < self.note.response_timeout {
            if self.poll()?.is_some() {
                return self.note.take_response()
            }

            delay.delay_ms(RESPONSE_DELAY);
            waited += RESPONSE_DELAY;
        }

        error!("response timed out (>= {}).", self.note.response_timeout);
        Err(NoteError::TimeOut)
    }

    /// Wait for response and return deserialized object.
    pub fn wait(mut self, delay: &mut impl DelayNs) -> Result<T, NoteError> {
        let mut waited = 0;

        while waited < self.note.response_timeout {
            if let Some(r) = self.poll()? {
                return Ok(r)
            }

            delay.delay_ms(RESPONSE_DELAY);
            waited += RESPONSE_DELAY;
        }

        error!("response timed out (>= {}).", self.note.response_timeout);
        Err(NoteError::TimeOut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal_mock::eh1::delay::StdSleep;
    use embedded_hal_mock::eh1::i2c::{Mock, Transaction};

    pub fn new_mock() -> Notecard<Mock> {
        // let exp = [ Transaction::write(0x17, vec![]) ];
        let i2c = Mock::new(&[]);
        Notecard::new(i2c)
    }

    #[test]
    fn resize_buf() {
        let c = new_mock();
        assert_eq!(c.buf.capacity(), DEFAULT_BUF_SIZE);

        let mut c = c.resize_buf::<1024>().unwrap();
        assert_eq!(c.buf.capacity(), 1024);

        c.i2c.done();
    }

    #[test]
    fn raw_request() {
        let mut expect = b"{\"req\":\"card.location\"}\n".to_vec();
        expect.insert(0, 24);
        let exp = [
            Transaction::write(0x17, vec![0, 0]),
            Transaction::read(0x17, vec![0, 0]),
            Transaction::write(0x17, expect),
        ];
        let i2c = Mock::new(&exp);
        let mut c: Notecard<Mock> = Notecard::new(i2c);
        let mut delay = StdSleep::new();
        c.request_raw(&mut delay, b"{\"req\":\"card.location\"}\n")
            .unwrap();

        c.i2c.done();
    }
}
