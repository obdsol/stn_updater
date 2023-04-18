use crate::codec::{Error, RequestFrame, ResponseFrame};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub trait IntoBytes {
    fn into_bytes(&self) -> Vec<u8>;
}

impl<T: Serialize> IntoBytes for T {
    fn into_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
}

pub trait Request: IntoBytes {
    const COMMAND: u8;
    type Response: Response;

    fn frame(&self) -> RequestFrame {
        RequestFrame::new(Self::COMMAND, self.into_bytes())
    }
}

pub trait Response: DeserializeOwned {
    fn from_frame<T: Request>(frame: ResponseFrame) -> Result<Self, Error> {
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

#[derive(Deserialize, Debug)]
pub struct ConnectResponse;
impl Response for ConnectResponse {}

#[derive(Serialize)]
pub struct GetVersionRequest;
impl Request for GetVersionRequest {
    const COMMAND: u8 = 0x06;
    type Response = GetVersionResponse;
}

#[derive(Deserialize, Debug)]
pub struct GetVersionResponse {
    major: u8,
    minor: u8,
}
impl Response for GetVersionResponse {}

#[derive(Serialize)]
pub struct GetDevIDRequest;
impl Request for GetDevIDRequest {
    const COMMAND: u8 = 0x07;
    type Response = GetDevIDResponse;
}

#[derive(Deserialize, Debug)]
pub struct GetDevIDResponse(u16);
impl Response for GetDevIDResponse {}

#[derive(Serialize)]
pub struct GetHWRevRequest;
impl Request for GetHWRevRequest {
    const COMMAND: u8 = 0x08;
    type Response = GetHWRevResponse;
}

#[derive(Deserialize, Debug)]
pub struct GetHWRevResponse {
    major: u8,
    minor: u8,
}
impl Response for GetHWRevResponse {}

#[derive(Serialize)]
pub struct GetSerialNumberRequest;
impl Request for GetSerialNumberRequest {
    const COMMAND: u8 = 0x0A;
    type Response = GetSerialNumberResponse;
}

#[derive(Deserialize, Debug)]
pub struct GetSerialNumberResponse {
    serial: [char; 8],
}
impl Response for GetSerialNumberResponse {}

#[derive(Serialize)]
pub struct GetDeviceNameRequest;
impl Request for GetDeviceNameRequest {
    const COMMAND: u8 = 0x0B;
    type Response = GetDeviceNameResponse;
}

#[derive(Deserialize, Debug)]
pub struct GetDeviceNameResponse {
    name: [char; 32],
}
impl Response for GetDeviceNameResponse {}

#[derive(Serialize)]
pub struct GetFWStatusRequest;
impl Request for GetFWStatusRequest {
    const COMMAND: u8 = 0x0F;
    type Response = GetFWStatusResponse;
}

#[derive(Deserialize, Debug)]
pub struct GetFWStatusResponse(u8);
impl Response for GetFWStatusResponse {}

pub struct StartUploadRequest {
    image_size: u32,
    mode: u8,
}
impl IntoBytes for StartUploadRequest {
    fn into_bytes(&self) -> Vec<u8> {
        let mut output = self.image_size.to_be_bytes()[1..].to_vec();
        output.push(self.mode);
        output
    }
}
impl Request for StartUploadRequest {
    const COMMAND: u8 = 0x30;
    type Response = StartUploadResponse;
}

#[derive(Deserialize, Debug)]
pub struct StartUploadResponse(u16);
impl Response for StartUploadResponse {}

pub struct SendChunkRequest {
    chunk_num: u16,
    data: Vec<u8>,
}
impl IntoBytes for SendChunkRequest {
    fn into_bytes(&self) -> Vec<u8> {
        let mut output = self.chunk_num.to_be_bytes().to_vec();
        output.extend_from_slice(&self.data);
        output
    }
}
impl Request for SendChunkRequest {
    const COMMAND: u8 = 0x31;
    type Response = SendChunkResponse;
}

#[derive(Deserialize, Debug)]
pub struct SendChunkResponse(u16);
impl Response for SendChunkResponse {}
