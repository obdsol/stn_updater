use std::time::Duration;

use futures::{sink::SinkExt, StreamExt};
use stn_updater::codec::{self, RequestFrame, ResponseFrame, StnCodec};
use stn_updater::protocol::{ConnectRequest, ConnectResponse, Request, Response};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::timeout;
use tokio_serial::SerialPortBuilderExt;
use tokio_util::codec::{Decoder, Encoder, Framed};

struct Updater<T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Encoder<RequestFrame> + Decoder<Item = ResponseFrame>,
{
    framed: Framed<T, U>,
}

impl<T, U> Updater<T, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    U: Encoder<RequestFrame, Error = codec::Error>
        + Decoder<Item = ResponseFrame, Error = codec::Error>,
{
    fn new(io: T, codec: U) -> Updater<T, U> {
        Updater {
            framed: codec.framed(io),
        }
    }

    async fn transmit<R: Request>(&mut self, request: R) -> Result<R::Response, codec::Error> {
        self.framed.send(request.frame()).await?;
        let response_frame = timeout(Duration::from_secs(3), self.framed.next())
            .await
            .unwrap()
            .unwrap()?;
        let response: R::Response = Response::from::<R>(response_frame)?;
        Ok(response)
    }
}

#[tokio::main]
async fn main() -> Result<(), codec::Error> {
    let serial_stream = tokio_serial::new("COM6", 115200).open_native_async()?;

    let mut updater = Updater::new(serial_stream, StnCodec::new());

    let response: ConnectResponse = updater.transmit(ConnectRequest).await?;

    Ok(())
}
