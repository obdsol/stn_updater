use crate::codec::{self, RequestFrame, ResponseFrame, StnCodec};
use crate::protocol::{ConnectRequest, ConnectResponse, Request, Response};
use futures::{sink::SinkExt, StreamExt};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time;
use tokio_util::codec::{Decoder, Encoder, Framed};

pub struct Updater<T, U>
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
    pub fn new(io: T, codec: U) -> Updater<T, U> {
        Updater {
            framed: codec.framed(io),
        }
    }

    pub async fn transmit<R: Request>(
        &mut self,
        request: R,
        timeout: Duration,
    ) -> Result<R::Response, codec::Error> {
        self.framed.send(request.frame()).await?;
        let response_frame = time::timeout(timeout, self.framed.next())
            .await
            .unwrap()
            .unwrap()?;
        let response: R::Response = Response::from_frame::<R>(response_frame)?;
        Ok(response)
    }
}
