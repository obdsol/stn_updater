use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use bytes::Buf;

pub struct FirmwareImageDescriptor {
    pub image_type: u8,
    pub next_idx: u8,
    pub error_idx: u8,
    pub image_offset: u32,
    pub image_size: u32,
}

pub struct FirmwareImage {
    pub device_ids: HashSet<u16>,
    pub descriptors: Vec<FirmwareImageDescriptor>,
    pub data: Vec<u8>,
}

impl FirmwareImage {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<FirmwareImage> {
        let firmware_file = fs::read(path)?;
        let mut buf: &[u8] = &firmware_file;

        if &buf[..6] != b"STNFWv" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file signature",
            ));
        }
        buf.advance(6);

        if &buf[..2] != b"05" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file version",
            ));
        }
        buf.advance(2);

        let device_ids_count = buf.get_u8();

        let device_ids = (0..device_ids_count)
            .map(|_| buf.get_u16())
            .collect::<HashSet<u16>>();

        let descriptor_count = buf.get_u8();

        let descriptors = if descriptor_count == 0 {
            vec![FirmwareImageDescriptor {
                image_type: 0x00,
                next_idx: 0xFF,
                error_idx: 0x00,
                image_offset: 12,
                image_size: (firmware_file.len() - 12) as u32,
            }]
        } else {
            (0..descriptor_count)
                .map(|_| {
                    let image_type = buf.get_u8();
                    let _ = buf.get_u8();
                    let next_idx = buf.get_u8();
                    let error_idx = buf.get_u8();
                    let image_offset = buf.get_u32();
                    let image_size = buf.get_u32();

                    FirmwareImageDescriptor {
                        image_type,
                        next_idx,
                        error_idx,
                        image_offset,
                        image_size,
                    }
                })
                .collect()
        };

        Ok(FirmwareImage {
            device_ids,
            descriptors,
            data: buf.to_vec(),
        })
    }
}
