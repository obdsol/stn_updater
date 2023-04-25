use crate::codec::{self, RequestFrame, ResponseFrame, SerialCodec};
use crate::protocol::{
    ConnectRequest, ConnectResponse, Request, ResetRequest, ResetResponse, Response,
};
use async_trait::async_trait;
use futures::{sink::SinkExt, StreamExt};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time;
use tokio_util::codec::{Decoder, Encoder, Framed};

#[async_trait]
pub trait Resetter {
    type Device;
    async fn reset(device: &mut Self::Device) -> Result<(), codec::Error>;
}

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

        let now = time::Instant::now();
        loop {
            let elapsed = now.elapsed();
            if elapsed >= timeout {
                self.framed.read_buffer_mut().clear();
                return Err(codec::Error::Timeout);
            }

            match tokio::time::timeout(timeout - elapsed, self.framed.next()).await {
                Ok(frame) => {
                    if let Some(Ok(response_frame)) = frame {
                        let response: R::Response = Response::from_frame::<R>(response_frame)?;
                        return Ok(response);
                    }
                }
                Err(_) => {
                    self.framed.read_buffer_mut().clear();
                    return Err(codec::Error::Timeout);
                }
            }
        }
    }

    pub async fn connect<D: Resetter<Device = T>>(&mut self) -> Result<(), codec::Error> {
        if let Ok(ConnectResponse) = self.transmit(ConnectRequest, Duration::from_secs(1)).await {
            return Ok(());
        } else {
            D::reset(self.framed.get_mut()).await?;
            for _ in 0..5 {
                if let Ok(ConnectResponse) =
                    self.transmit(ConnectRequest, Duration::from_secs(1)).await
                {
                    return Ok(());
                }
            }
            return Err(codec::Error::Timeout);
        }
    }

    pub async fn reset(&mut self) -> Result<(), codec::Error> {
        let _ = self.transmit(ResetRequest, Duration::from_secs(1)).await?;
        Ok(())
    }
}
