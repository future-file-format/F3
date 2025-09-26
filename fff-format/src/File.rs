#![allow(clippy::all)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(unused)]
#![allow(improper_ctypes)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::use_self)]

include!(concat!(env!("OUT_DIR"), "/File.rs"));

impl From<self::fff::flatbuf::CompressionType> for u8 {
    fn from(t: self::fff::flatbuf::CompressionType) -> u8 {
        t.0
    }
}

impl From<u8> for self::fff::flatbuf::CompressionType {
    fn from(t: u8) -> self::fff::flatbuf::CompressionType {
        self::fff::flatbuf::CompressionType(t)
    }
}
