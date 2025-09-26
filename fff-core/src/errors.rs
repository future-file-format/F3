use std::{
    fmt::{Display, Formatter},
    io, result,
};

use arrow_schema::ArrowError;
use flatbuffers::InvalidFlatbuffer;
use snafu::Location;
use vortex_error::VortexError;

/// Derived from parquet-rs
#[derive(Debug)]
pub enum Error {
    /// General error.
    /// Returned when code violates normal workflow of working with files.
    General(String),
    /// "Not yet implemented" error.
    /// Returned when functionality is not yet available.
    NYI(String),
    /// "End of file" error.
    /// Returned when IO related failures occur, e.g. when there are not enough bytes to
    /// decode.
    EOF(String),
    IndexOutOfBound(usize, usize),
    ParseError(String),
    IO(String, Location),
    /// An external error variant
    External(Box<dyn std::error::Error + Send + Sync>),
    CastSliceError(String),
    ObjectStore(object_store::Error),
}

pub type Result<T, E = Error> = result::Result<T, E>;

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::External(Box::new(e))
    }
}

impl From<ArrowError> for Error {
    fn from(e: ArrowError) -> Error {
        Error::External(Box::new(e))
    }
}

impl From<VortexError> for Error {
    fn from(e: VortexError) -> Error {
        Error::External(Box::new(e))
    }
}

impl From<bytemuck::PodCastError> for Error {
    fn from(e: bytemuck::PodCastError) -> Error {
        Error::CastSliceError(e.to_string())
    }
}

impl From<InvalidFlatbuffer> for Error {
    fn from(e: InvalidFlatbuffer) -> Error {
        Error::ParseError(e.to_string())
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Error {
        Error::General(e.to_string())
    }
}

/// Create a "Not Yet Implemented" error with a message
pub fn nyi_err<T>(msg: impl Into<String>) -> Error {
    Error::NYI(msg.into())
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::General(source) => write!(f, "General error: {}", source),
            Error::NYI(source) => write!(f, "Not yet implemented: {}", source),
            Error::EOF(source) => write!(f, "End of file: {}", source),
            Error::IndexOutOfBound(index, size) => {
                write!(f, "Index out of bound: {} >= {}", index, size)
            }
            Error::ParseError(source) => write!(f, "Parse error: {}", source),
            Error::IO(source, location) => write!(f, "IO error: {} at {}", source, location),
            Error::External(source) => write!(f, "External error: {}", source),
            Error::CastSliceError(source) => write!(f, "Cast slice error: {}", source),
            Error::ObjectStore(source) => write!(f, "Object store error: {}", source),
        }
    }
}

/// A macro to simplify common error handling patterns
#[macro_export]
macro_rules! general_error {
    ($msg:expr) => {
        $crate::errors::Error::General($msg.into())
    };
    ($msg:expr, $err:expr) => {
        $crate::errors::Error::General(format!("{}: {}", $msg, $err))
    };
}

/// A macro to simplify "Not Yet Implemented" error handling patterns
#[macro_export]
macro_rules! nyi_err {
    ($msg:expr) => {
        Err($crate::errors::Error::NYI($msg.into()))
    };
}
