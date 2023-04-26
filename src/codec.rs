use crate::error::Error;

use bytes::{Buf, BufMut, BytesMut};
use crc::Crc;
use tokio_util::codec::{Decoder, Encoder};

// #[derive(Debug)]
// pub enum Error {
//     IOError(std::io::Error),
//     InvalidCommand(ResponseFrame),
//     InvalidResponse(ResponseFrame),
//     BinCode(Box<bincode::ErrorKind>),
//     SerialPortError(tokio_serial::Error),
//     Timeout,
//     Placeholder,
// }

// impl From<std::io::Error> for Error {
//     fn from(err: std::io::Error) -> Error {
//         Error::IOError(err)
//     }
// }

// impl From<Box<bincode::ErrorKind>> for Error {
//     fn from(err: Box<bincode::ErrorKind>) -> Error {
//         Error::BinCode(err)
//     }
// }

// impl From<tokio_serial::Error> for Error {
//     fn from(err: tokio_serial::Error) -> Error {
//         Error::SerialPortError(err)
//     }
// }

// impl From<Elapsed> for Error {
//     fn from(_err: Elapsed) -> Error {
//         Error::Timeout
//     }
// }

#[derive(Debug, PartialEq, Eq)]
pub struct RequestFrame {
    pub command: u8,
    pub data: Vec<u8>,
}

impl RequestFrame {
    pub const fn new(command: u8, data: Vec<u8>) -> RequestFrame {
        RequestFrame { command, data }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResponseFrame {
    pub ack: bool,
    pub command: u8,
    pub data: Vec<u8>,
}

impl ResponseFrame {
    pub const fn new(ack: bool, command: u8, data: Vec<u8>) -> ResponseFrame {
        ResponseFrame { ack, command, data }
    }
}

pub struct SerialCodec {
    crc: Crc<u16>,
}

impl SerialCodec {
    pub const STX: u8 = 0x55;
    pub const ETX: u8 = 0x04;
    pub const DLE: u8 = 0x05;

    pub const fn new() -> SerialCodec {
        SerialCodec {
            crc: Crc::<u16>::new(&crc::CRC_16_XMODEM),
        }
    }

    fn byte_stuff(data: u8, dst: &mut BytesMut) {
        if let SerialCodec::STX | SerialCodec::ETX | SerialCodec::DLE = data {
            dst.put_u8(SerialCodec::DLE);
        }
        dst.put_u8(data);
    }
}

impl Encoder<RequestFrame> for SerialCodec {
    type Error = Error;

    fn encode(&mut self, item: RequestFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let mut digest = self.crc.digest();

        dst.put_u8(SerialCodec::STX);
        dst.put_u8(SerialCodec::STX);

        SerialCodec::byte_stuff(item.command, dst);
        digest.update(&[item.command]);

        let length = (item.data.len() as u16).to_be_bytes();
        SerialCodec::byte_stuff(length[0], dst);
        SerialCodec::byte_stuff(length[1], dst);
        digest.update(&length);

        for data in &item.data {
            SerialCodec::byte_stuff(*data, dst);
        }
        digest.update(&item.data);

        let crc = digest.finalize().to_be_bytes();
        SerialCodec::byte_stuff(crc[0], dst);
        SerialCodec::byte_stuff(crc[1], dst);

        dst.put_u8(SerialCodec::ETX);

        Ok(())
    }
}

impl Decoder for SerialCodec {
    type Item = ResponseFrame;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            return Ok(None);
        }

        if src[..2] != [SerialCodec::STX, SerialCodec::STX] {
            return Err(Error::IOError(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("STX: {:?}", &src[..2]),
            )));
        }

        let mut digest = self.crc.digest();
        let mut skip = false;

        let mut data = vec![];

        for idx in 2..src.len() {
            if skip {
                skip = false;
                data.push(src[idx]);
            } else {
                match src[idx] {
                    SerialCodec::STX => {
                        return Err(Error::IOError(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Unexpected STX: {:?}", &src[..idx + 1]),
                        )));
                    }
                    SerialCodec::ETX => {
                        if data.len() < 4 || (data[1] as usize) != (data.len() - 4) {
                            return Err(Error::IOError(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("Bad frame: {:?}", &src[..idx + 1]),
                            )));
                        }

                        digest.update(&data);
                        if digest.finalize() != 0 {
                            return Err(Error::IOError(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("Bad CRC: {:?}", &src[..idx + 1]),
                            )));
                        }

                        let ack = (data[0] & 0x40) == 0x40;
                        let command = data.remove(0) & 0x3F;
                        let length = data.remove(0) as usize;

                        data.truncate(length);

                        let response = ResponseFrame { ack, command, data };

                        src.advance(idx + 1);

                        return Ok(Some(response));
                    }
                    SerialCodec::DLE => skip = true,
                    _ => {
                        data.push(src[idx]);
                    }
                }
            }
        }

        Ok(None)
    }
}
