use stn_updater::codec::{RequestFrame, ResponseFrame, SerialCodec};
use tokio_util::codec::{Decoder, Encoder};

use test_case::test_case;

#[test_case(RequestFrame::new(0x03, vec![]), &[SerialCodec::STX, SerialCodec::STX, 0x03, 0x00, 0x00, 0x59, 0x50, SerialCodec::ETX])]
#[test_case(
    RequestFrame::new(0x31, vec![0x00, 0x00, 0x05, 0x05, 0x03]),
    &[
        SerialCodec::STX, SerialCodec::STX,
        0x31,
        0x00, SerialCodec::DLE, 0x05,
        0x00, 0x00,
        SerialCodec::DLE, 0x05,
        SerialCodec::DLE, 0x05,
        0x03,
        0x66, 0x68,
        SerialCodec::ETX
    ]
)]
fn test_encoder(request: RequestFrame, bytes: &[u8]) {
    let mut codec = SerialCodec::new();

    let mut buf = bytes::BytesMut::new();
    codec.encode(request, &mut buf).unwrap();

    assert_eq!(&buf as &[u8], bytes);
}

#[test_case(&[0x55, 0x55, 0x46, 0x02, SerialCodec::DLE, 0x04, 0x01, 0xFB, 0x80, SerialCodec::ETX], ResponseFrame::new(true, 0x06, vec![0x04, 0x01]))]
fn test_decoder(data: &[u8], response: ResponseFrame) {
    let mut codec = SerialCodec::new();
    let mut buf = bytes::BytesMut::from(data);

    assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), response);
}
