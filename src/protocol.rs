use crate::codec::{Error, RequestFrame, ResponseFrame};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

trait StnSerialize {
    fn serialize(&self) -> Vec<u8>;
}

impl <T: Serialize> StnSerialize for T {
    fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
}

trait StnRequest<'de>: StnSerialize {
    const COMMAND: u8;
    type Response: StnResponse<'de>;

    fn frame(&self) -> RequestFrame {
        RequestFrame::new(Self::COMMAND, self.serialize())
    }
}

trait StnResponse<'de>: DeserializeOwned {
    fn from<T: StnRequest<'de>>(frame: ResponseFrame) -> Result<Self, Error> {
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
struct ConnectRequest;
impl StnRequest<'_> for ConnectRequest {
    const COMMAND: u8 = 0x03;
    type Response = ConnectResponse;
}

#[derive(Deserialize)]
struct ConnectResponse;
impl StnResponse<'_> for ConnectResponse {}