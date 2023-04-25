use crate::codec::{self, RequestFrame, ResponseFrame};
use crate::firmware::FirmwareImage;
use crate::protocol::{
    ConnectRequest, ConnectResponse, GetDevIDRequest, GetDevIDResponse, GetHWRevRequest,
    GetHWRevResponse, GetSerialNumberRequest, GetSerialNumberResponse, Request, ResendLastRequest,
    ResetRequest, Response, SendChunkRequest, SendChunkResponse, StartUploadRequest,
    StartUploadResponse,
};
use async_trait::async_trait;
use futures::{sink::SinkExt, StreamExt};
use indicatif::ProgressBar;
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
    connect_retry: usize,
    resend_retry: usize,
    connect_timeout: Duration,
    request_timeout: Duration,
    chunk_timeout: Duration,
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
            connect_retry: 5,
            resend_retry: 5,
            connect_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_millis(200),
            chunk_timeout: Duration::from_secs(5),
        }
    }

    async fn inner_recv_response<R: Request>(
        &mut self,
        timeout: Duration,
    ) -> Result<R::Response, codec::Error> {
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

    pub async fn recv_response<R: Request>(
        &mut self,
        timeout: Duration,
    ) -> Result<R::Response, codec::Error> {
        let mut response = self.inner_recv_response::<R>(timeout).await;

        let mut index = 0;
        while let Err(codec::Error::Timeout) = response {
            self.framed
                .send(ResendLastRequest::<R::Response>::new().frame())
                .await?;
            response = self.inner_recv_response::<R>(timeout).await;

            index += 1;
            if index >= self.resend_retry {
                break;
            }
        }

        response
    }

    pub async fn transmit<R: Request>(
        &mut self,
        request: R,
        timeout: Duration,
    ) -> Result<R::Response, codec::Error> {
        self.framed.send(request.frame()).await?;
        self.recv_response::<R>(timeout).await
    }

    pub async fn connect<D: Resetter<Device = T>>(&mut self) -> Result<(), codec::Error> {
        if let Ok(ConnectResponse) = self.transmit(ConnectRequest, self.connect_timeout).await {
            Ok(())
        } else {
            D::reset(self.framed.get_mut()).await?;
            for _ in 0..self.connect_retry {
                if let Ok(ConnectResponse) = self
                    .transmit(ConnectRequest, Duration::from_millis(50))
                    .await
                {
                    return Ok(());
                }
            }
            Err(codec::Error::Timeout)
        }
    }

    pub async fn device_id(&mut self) -> Result<u16, codec::Error> {
        let GetDevIDResponse(device_id) =
            self.transmit(GetDevIDRequest, self.request_timeout).await?;
        Ok(device_id)
    }

    pub async fn serial_number(&mut self) -> Result<String, codec::Error> {
        let GetSerialNumberResponse { serial } = self
            .transmit(GetSerialNumberRequest, self.request_timeout)
            .await?;
        Ok(String::from_utf8_lossy(&serial).to_string())
    }

    pub async fn hw_version(&mut self) -> Result<(u8, u8), codec::Error> {
        let GetHWRevResponse { major, minor } =
            self.transmit(GetHWRevRequest, self.request_timeout).await?;
        Ok((major, minor))
    }

    pub async fn start_upload(&mut self, image_size: u32) -> Result<u16, codec::Error> {
        let StartUploadResponse(max_chunk_size) = self
            .transmit(
                StartUploadRequest {
                    image_size,
                    mode: 1,
                },
                self.request_timeout,
            )
            .await?;
        Ok(max_chunk_size)
    }

    pub async fn send_chunk(&mut self, index: usize, chunk: &[u8]) -> Result<u16, codec::Error> {
        let mut error = codec::Error::Placeholder;
        for _ in 0..5 {
            match self
                .transmit(
                    SendChunkRequest {
                        chunk_num: index as u16,
                        data: chunk.to_vec(),
                    },
                    self.chunk_timeout,
                )
                .await
            {
                Ok(SendChunkResponse(response_index)) => {
                    return Ok(response_index);
                }
                Err(err) => {
                    error = err;
                }
            }
        }
        Err(error)
    }

    pub async fn reset(&mut self) -> Result<(), codec::Error> {
        let _ = self.transmit(ResetRequest, self.request_timeout).await?;
        Ok(())
    }

    pub async fn upload_firmware<D: Resetter<Device = T>>(
        &mut self,
        firmware: FirmwareImage,
    ) -> Result<(), codec::Error> {
        self.connect::<D>().await?;
        let device_id = dbg!(self.device_id().await?);

        if firmware.device_ids.contains(&device_id) {
            dbg!(self.serial_number().await?);
            dbg!(self.hw_version().await?);

            let mut image_idx = 0;

            loop {
                let descriptor = &firmware.descriptors[image_idx];
                let offset = descriptor.image_offset as usize;
                let size = descriptor.image_size as usize;
                let firmware_data = &firmware.data[offset..offset + size];

                // Rounded down to the nearest multiple of 16
                let _ = (dbg!(self.start_upload(firmware_data.len() as u32).await?) & !15) as usize;
                let chunk_size = 1024;
                let num_chunks = (firmware_data.len() + chunk_size - 1) / chunk_size;

                let pb = ProgressBar::new(num_chunks as u64);
                for (idx, chunk) in pb.wrap_iter(firmware_data.chunks(chunk_size).enumerate()) {
                    for _ in 0..3 {
                        let chunk_idx = self.send_chunk(idx, chunk).await?;
                        if idx == chunk_idx as usize {
                            break;
                        }
                    }
                }

                if descriptor.next_idx != 0xFF {
                    match descriptor.image_type {
                        // Normal
                        0x00 => {
                            image_idx = descriptor.next_idx as usize;
                        }

                        // Normal, Tolerate Errors
                        0x01 => {
                            // TODO: Implement
                        }

                        // Validation
                        0x10 => {
                            // TODO: Implement
                        }

                        _ => unreachable!(),
                    }
                } else {
                    break;
                }
            }
        }

        self.reset().await?;

        Ok(())
    }
}
