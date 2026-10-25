use embedded_hal_async::delay::DelayNs;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::ChannelAsync;

use super::{FutureResponse, NoteError, Notecard};

pub struct Env<'a, IOM: ChannelAsync, const BS: usize> {
    note: &'a mut Notecard<IOM, BS>,
}

impl<'a, IOM: ChannelAsync, const BS: usize> Env<'a, IOM, BS> {
    pub fn from(note: &mut Notecard<IOM, BS>) -> Env<'_, IOM, BS> {
        Env { note }
    }

    pub async fn get<T: DeserializeOwned>(
        self,
        delay: &mut impl DelayNs,
    ) -> Result<FutureResponse<'a, res::Get<T>, IOM, BS>, NoteError> {
        self.note.request(
            delay,
            req::Get {
                req: "env.get",
                name: None,
                time: None,
            },
        ).await?;

        Ok(FutureResponse::from(self.note))
    }
}

pub mod req {
    use super::*;

    #[derive(Serialize, Debug)]
    pub struct Get<'a> {
        pub req: &'a str,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub name: Option<&'a str>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub time: Option<u64>,
    }
}

pub mod res {
    use super::*;

    #[derive(Deserialize, Debug)]
    pub struct Get<T> {
        pub body: T,
        pub time: u64,
    }
}
