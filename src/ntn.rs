//! https://dev.blues.io/api-reference/notecard-api/ntn-requests/

#[allow(unused_imports)]
use defmt::{debug, error, info, trace, warn};
use embedded_hal_async::delay::DelayNs;
use serde::{Deserialize, Serialize};

use crate::channel::asynch::Channel;

use super::{FutureResponse, NoteError, Notecard};

pub struct NTN<'a, IOM: Channel, D: DelayNs, const BS: usize> {
    note: &'a mut Notecard<IOM, D, BS>,
}

#[derive(Clone, Copy, defmt::Format)]
pub enum NtnSetGps {
    /// Use notecard gps on starnote as well
    Notecard,

    /// Use starnotes own gps (default).
    Starnote,
}

impl<'a, IOM: Channel, D: DelayNs, const BS: usize> NTN<'a, IOM, D, BS> {
    pub fn from(note: &mut Notecard<IOM, D, BS>) -> NTN<'_, IOM, D, BS> {
        NTN { note }
    }

    /// Once a Notecard is connected to a Starnote device, the presence of a physical Starnote is stored in a permanent configuration that is not affected by a card.restore request. This request clears this configuration and allows you to return to testing NTN mode over cellular or Wi-Fi.
    pub async fn reset<const PS: usize>(
        self,
    ) -> Result<FutureResponse<'a, res::Empty, IOM, D, BS>, NoteError> {
        self.note.request_raw(b"{\"req\":\"ntn.reset\"}\n").await?;
        Ok(FutureResponse::from(self.note))
    }

    /// Gets and sets the background download status of MCU host or Notecard
    /// firmware updates.
    pub async fn status(
        self,
    ) -> Result<FutureResponse<'a, res::Status, IOM, D, BS>, NoteError> {
        self.note
            .request_raw(b"{\"req\":\"ntn.status\"}\n").await?;

        Ok(FutureResponse::from(self.note))
    }

    /// Determines whether a Notecard should override a paired Starnote's GPS/GNSS location with its own GPS/GNSS location. The paired Starnote uses its own GPS/GNSS location by default.
    pub async fn gps(
        self,
        gps: Option<NtnSetGps>,
    ) -> Result<FutureResponse<'a, res::Gps, IOM, D, BS>, NoteError> {
        self.note.request(
            req::Gps {
                req: "ntn.gps",
                on: gps.map(|g| matches!(g, NtnSetGps::Notecard)),
                off: gps.map(|g| matches!(g, NtnSetGps::Starnote)),
            },
        ).await?;
        Ok(FutureResponse::from(self.note))
    }
}

pub mod req {
    use super::*;

    #[derive(Deserialize, Serialize, Debug, defmt::Format, Default)]
    pub struct Gps {
        pub req: &'static str,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub on: Option<bool>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub off: Option<bool>,
    }
}

pub mod res {
    use super::*;

    #[derive(Deserialize, Debug, defmt::Format)]
    pub struct Empty {}

    #[derive(Deserialize, Serialize, Debug, defmt::Format, Default)]
    pub struct Gps {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub on: Option<bool>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub off: Option<bool>,
    }

    #[derive(Deserialize, Debug, defmt::Format)]
    pub struct Status {
        pub err: Option<heapless::String<120>>,
        pub status: Option<heapless::String<120>>,
    }
}

// #[cfg(test)]
// mod tests {
//  use super::*;
// }
