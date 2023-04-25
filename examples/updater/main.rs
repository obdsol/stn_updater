use std::io::{self, Write};
use std::time::{self, Duration};

use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use futures::StreamExt;
use stn_updater::codec::{self, SerialCodec};
use stn_updater::updater::{Resetter, Updater};
use tokio::time::timeout;
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialStream};
use tokio_util::codec::Decoder;

struct EndingCodec {
    ending: Vec<u8>,
}

impl EndingCodec {
    fn new<S: AsRef<str>>(ending: S) -> EndingCodec {
        EndingCodec {
            ending: ending.as_ref().as_bytes().to_vec(),
        }
    }
}

impl Decoder for EndingCodec {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let ending_len = self.ending.len();
        if src.len() < ending_len {
            return Ok(None);
        } else {
            match src.windows(ending_len).position(|w| w == &self.ending) {
                Some(position) => {
                    let frame = src[..position + ending_len].to_vec();
                    src.advance(frame.len());
                    return Ok(Some(frame));
                }
                None => {
                    return Ok(None);
                }
            }
        }
    }
}

async fn read_until<S: AsRef<str>>(
    device: &mut SerialStream,
    ending: S,
    timeout: Duration,
) -> Result<String, codec::Error> {
    let mut stream = EndingCodec::new(ending).framed(device);
    let now = time::Instant::now();
    loop {
        if now.elapsed() >= timeout {
            return Err(codec::Error::Timeout);
        }

        if let Some(Ok(response)) = stream.next().await {
            return Ok(std::str::from_utf8(&response).unwrap().to_string());
        }
    }
}

struct ATZResetter;
#[async_trait]
impl Resetter for ATZResetter {
    type Device = SerialStream;
    async fn reset(device: &mut Self::Device) -> Result<(), codec::Error> {
        device.clear(tokio_serial::ClearBuffer::All)?;

        device.try_write(b"?\r")?;
        let _ = read_until(device, ">", Duration::from_secs(1)).await?;

        device.try_write(b"ATZ\r")?;
        let _ = read_until(device, "ATZ\r", Duration::from_secs(1)).await?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), codec::Error> {
    let serial_stream = tokio_serial::new("COM8", 460800)
        .flow_control(tokio_serial::FlowControl::Hardware)
        .timeout(Duration::from_secs(1))
        .open_native_async()?;

    let mut updater = Updater::new(serial_stream, SerialCodec::new());
    updater.connect::<ATZResetter>().await?;
    updater.reset().await?;

    Ok(())
}
