use crate::codec::ResponseFrame;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error occurred")]
    IOError(#[from] std::io::Error),

    #[error("InvalidCommand")]
    InvalidCommand(ResponseFrame),

    #[error("InvalidResponse")]
    InvalidResponse(ResponseFrame),

    #[error("BinCode")]
    BinCode(#[from] Box<bincode::ErrorKind>),

    #[error("Timeout")]
    Timeout,

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("Placeholder")]
    Placeholder,
}
