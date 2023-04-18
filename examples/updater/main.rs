use stn_updater::codec::{self, StnCodec};
use stn_updater::updater::Updater;
use tokio_serial::SerialPortBuilderExt;

use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), codec::Error> {
    let serial_stream = tokio_serial::new("COM6", 115200).open_native_async()?;

    let mut updater = Updater::new(serial_stream, StnCodec::new());

    Ok(())
}
