use crate::codec::{Error, RequestFrame, ResponseFrame};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub trait Request: Serialize {
    const COMMAND: u8;
    type Response: Response;

    fn frame(&self) -> RequestFrame {
        RequestFrame::new(Self::COMMAND, bincode::serialize(self).unwrap())
    }
}

pub trait Response: DeserializeOwned {
    fn from<T: Request>(frame: ResponseFrame) -> Result<Self, Error> {
        if frame.command != T::COMMAND {
            Err(Error::InvalidCommand(frame))
        } else if !frame.ack {
            Err(Error::InvalidResponse(frame))
        } else {
            Ok(bincode::deserialize(&frame.data)?)
        }
    }
}

#[derive(Serialize)]
pub struct ConnectRequest;
impl Request for ConnectRequest {
    const COMMAND: u8 = 0x03;
    type Response = ConnectResponse;
}

#[derive(Deserialize)]
pub struct ConnectResponse;
impl Response for ConnectResponse {}
