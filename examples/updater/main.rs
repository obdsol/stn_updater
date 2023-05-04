use std::collections::VecDeque;
use std::io::{self};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{self, Duration};

use async_trait::async_trait;
use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use bytes::{Buf, BytesMut};
use clap::{ArgGroup, Parser};
use futures::{Future, FutureExt, Stream, StreamExt};
use pin_project::pin_project;
use stn_updater::codec::SerialCodec;
use stn_updater::firmware;
use stn_updater::updater::{Resetter, Updater};

use terminal_menu as tm;
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialStream};
use tokio_util::codec::{Decoder, FramedRead};

use indicatif::ProgressBar;
use uuid::Uuid;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const UART_SERVICE_UUID: Uuid = Uuid::from_u128(0x0000FFF0_0000_1000_8000_00805F9B34FB);
const UART_RX_CHAR_UUID: Uuid = Uuid::from_u128(0x0000FFF1_0000_1000_8000_00805F9B34FB);
const UART_TX_CHAR_UUID: Uuid = Uuid::from_u128(0x0000FFF2_0000_1000_8000_00805F9B34FB);

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
            Ok(None)
        } else {
            match src.windows(ending_len).position(|w| w == &self.ending) {
                Some(position) => {
                    let frame = src[..position + ending_len].to_vec();
                    src.advance(frame.len());
                    Ok(Some(frame))
                }
                None => Ok(None),
            }
        }
    }
}

async fn read_until<D: tokio::io::AsyncRead + Unpin, S: AsRef<str>>(
    device: &mut D,
    ending: S,
    timeout: Duration,
) -> Result<String, stn_updater::error::Error> {
    let mut stream = FramedRead::new(device, EndingCodec::new(ending));
    let now = time::Instant::now();
    loop {
        if now.elapsed() >= timeout {
            return Err(stn_updater::error::Error::Timeout);
        }

        if let Some(Ok(response)) = stream.next().await {
            return Ok(std::str::from_utf8(&response).unwrap().to_string());
        }
    }
}

struct SerialATZResetter;
#[async_trait]
impl Resetter for SerialATZResetter {
    type Device = SerialStream;
    async fn reset(device: &mut Self::Device) -> anyhow::Result<()> {
        device.clear(tokio_serial::ClearBuffer::All)?;

        device.try_write(b"?\r")?;
        let _ = read_until(device, ">", Duration::from_secs(1)).await?;

        device.try_write(b"ATZ\r")?;
        let _ = read_until(device, "ATZ\r", Duration::from_secs(1)).await?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(())
    }
}

struct BLEATZResetter<'a> {
    _marker: PhantomData<PeripheralStream<'a>>,
}
#[async_trait]
impl<'a> Resetter for BLEATZResetter<'a> {
    type Device = PeripheralStream<'a>;

    async fn reset(device: &mut Self::Device) -> anyhow::Result<()> {
        device.rx_buffer.clear();

        device
            .periph
            .write(&device.char_tx, b"?\r", WriteType::WithResponse)
            .await?;
        let _ = read_until(device, ">", Duration::from_secs(1)).await?;

        device
            .periph
            .write(&device.char_tx, b"ATZ\r", WriteType::WithResponse)
            .await?;
        let _ = read_until(device, "ATZ\r", Duration::from_secs(1)).await?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(())
    }
}

impl<'a> PeripheralStream<'a> {
    fn new(
        periph: Peripheral,
        service_uuid: Uuid,
        rx_char_uuid: Uuid,
        tx_char_uuid: Uuid,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Self, anyhow::Error>> + 'a>> {
        Box::pin(async move {
            periph.connect().await?;
            periph.discover_services().await?;

            let mut char_rx = None;
            let mut char_tx = None;

            for service in periph.services() {
                if service.uuid == service_uuid {
                    for characteristic in service.characteristics {
                        if characteristic.uuid == rx_char_uuid
                            && characteristic.properties.contains(CharPropFlags::NOTIFY)
                        {
                            periph.subscribe(&characteristic).await?;
                            char_rx = Some(characteristic);
                        } else if characteristic.uuid == tx_char_uuid {
                            char_tx = Some(characteristic);
                        }
                    }
                }
            }

            let rx_stream = periph.notifications().await?;

            Ok(PeripheralStream {
                periph,
                char_rx: char_rx.unwrap(),
                char_tx: char_tx.unwrap(),
                rx_stream,
                rx_buffer: VecDeque::new(),
                tx_write_task: None,
            })
        })
    }
}

#[pin_project]
struct PeripheralStream<'a> {
    periph: Peripheral,
    char_rx: Characteristic,
    char_tx: Characteristic,
    rx_stream: Pin<Box<dyn Stream<Item = ValueNotification> + Send>>,
    rx_buffer: VecDeque<u8>,
    #[pin]
    tx_write_task: Option<CharWriteTask<'a>>,
}

struct CharWriteTask<'a> {
    _periph: Pin<Box<Peripheral>>,
    _characteristic: Pin<Box<Characteristic>>,
    _buffer: Pin<Box<[u8]>>,
    future: Pin<Box<dyn futures::Future<Output = Result<(), btleplug::Error>> + Send + 'a>>,
}

impl<'a> CharWriteTask<'a> {
    fn new(
        periph: Peripheral,
        characteristic: Characteristic,
        buffer: &[u8],
        write_type: WriteType,
    ) -> CharWriteTask<'a> {
        let periph = Box::pin(periph);
        let characteristic = Box::pin(characteristic);
        let buffer = Pin::new(buffer.to_vec().into_boxed_slice());
        let periph_ptr = periph.as_ref().get_ref() as *const _;
        let characteristic_ptr = characteristic.as_ref().get_ref() as *const _;
        let buffer_ptr = buffer.as_ref().get_ref() as *const _;

        let future = unsafe {
            Peripheral::write(&*periph_ptr, &*characteristic_ptr, &*buffer_ptr, write_type)
        };

        CharWriteTask {
            _periph: periph,
            _characteristic: characteristic,
            _buffer: buffer,
            future,
        }
    }
}

impl<'a> Future for CharWriteTask<'a> {
    type Output = Result<(), btleplug::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.future.poll_unpin(cx)
    }
}

impl<'a> AsyncWrite for PeripheralStream<'a> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let mut this = self.as_mut().project();

        if this.tx_write_task.is_none() {
            let write_task = CharWriteTask::new(
                this.periph.clone(),
                this.char_tx.clone(),
                buf,
                WriteType::WithoutResponse,
            );
            this.tx_write_task.set(Some(write_task));
        }

        match this.tx_write_task.as_mut().as_pin_mut().unwrap().poll(cx) {
            Poll::Ready(Ok(_)) => {
                this.tx_write_task.set(None);
                Poll::Ready(Ok(buf.len()))
            }
            Poll::Ready(Err(e)) => {
                this.tx_write_task.set(None);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl<'a> AsyncRead for PeripheralStream<'a> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let stream = self.get_mut();
        match stream.rx_stream.poll_next_unpin(cx) {
            std::task::Poll::Ready(Some(data)) => {
                stream.rx_buffer.extend(data.value);
            }
            _ => {}
        }
        let amount = stream.rx_buffer.len();
        if stream.rx_buffer.len() > 0 {
            let data = stream.rx_buffer.drain(0..amount).collect::<Vec<_>>();
            buf.put_slice(&data);
            return std::task::Poll::Ready(Ok(()));
        } else {
            return std::task::Poll::Pending;
        }
    }
}

#[derive(Parser, Debug)]
#[clap(group = ArgGroup::new("comms").args(&["port", "ble"]).required(true))]
#[clap(group = ArgGroup::new("serial").args(&["port", "baud", "flow-control"]).multiple(true))]
struct Args {
    /// Path to firmware image
    #[clap(parse(from_os_str), required = true)]
    firmware: PathBuf,

    /// Serial port
    #[clap(long, short, group = "serial", requires = "baud")]
    port: Option<String>,

    /// Baudrate
    #[clap(long, short, group = "serial")]
    baud: Option<u32>,

    /// Hardware flow-control
    #[clap(long, short, group = "serial")]
    flow_control: bool,

    /// Connect to BLE device
    #[clap(long)]
    ble: bool,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    let firmware = firmware::FirmwareImage::open(args.firmware)?;

    if let (Some(port), Some(baud)) = (args.port, args.baud) {
        let serial_stream = tokio_serial::new(port, baud)
            .timeout(Duration::from_secs(1))
            .open_native_async()?;

        let pb = ProgressBar::new(100);

        let mut updater = Updater::new(serial_stream, SerialCodec::new());
        updater
            .upload_firmware::<SerialATZResetter>(firmware, |idx, length| {
                pb.set_length(length as u64);
                pb.set_position(idx as u64);
            })
            .await?;
    } else if args.ble {
        let mut menu_items = vec![
            tm::label("-------------"),
            tm::label("Select Device"),
            tm::label("-------------"),
        ];

        let manager = Manager::new().await?;
        let adapter_list = manager.adapters().await?;

        if adapter_list.is_empty() {
            panic!("No Bluetooth adapters found");
        }

        let adapter = &adapter_list[0];

        adapter
            .start_scan(ScanFilter {
                services: vec![UART_SERVICE_UUID],
            })
            .await
            .expect("Can't scan BLE adapter for connected devices...");

        tokio::time::sleep(Duration::from_secs(6)).await;

        let peripherals = adapter.peripherals().await?;
        let mut uart_peripherals = vec![];
        for peripheral in peripherals.iter() {
            let properties = peripheral.properties().await?.unwrap();
            let local_name = properties
                .local_name
                .unwrap_or(String::from("(peripheral name unknown)"));
            let services = properties.services;
            if services.contains(&UART_SERVICE_UUID) {
                menu_items.push(tm::button(local_name));
                uart_peripherals.push(peripheral);
            }
        }

        if peripherals.len() > 0 {
            let menu = tm::menu(menu_items);
            tm::run(&menu);
            let peripheral = uart_peripherals.remove(tm::mut_menu(&menu).selected_item_index() - 3);
            let periph = PeripheralStream::new(
                peripheral.clone(),
                UART_SERVICE_UUID,
                UART_RX_CHAR_UUID,
                UART_TX_CHAR_UUID,
            )
            .await?;

            let pb = ProgressBar::new(100);

            let mut updater = Updater::new(periph, SerialCodec::new());
            updater
                .upload_firmware::<BLEATZResetter>(firmware, |idx, length| {
                    pb.set_length(length as u64);
                    pb.set_position(idx as u64);
                })
                .await?;
        }
    }

    Ok(())
}
