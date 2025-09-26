use flatbuffers::{FlatBufferBuilder, WIPOffset};

pub mod File;

pub const MAJOR_VERSION: u16 = 0;
pub const MINOR_VERSION: u16 = 1;
pub const MAGIC: &[u8; 2] = b"F3";
pub const POSTSCRIPT_SIZE: u64 = 32;

pub trait ToFlatBuffer {
    type Target<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>>;
}
