#![feature(new_range_api)]
use mimalloc::MiMalloc;

pub mod common;
mod compression;
pub mod counter;
pub mod file;
pub mod io;
pub mod options;
pub mod reader;
pub mod writer;

pub mod context;
pub mod decoder;
mod dict;
pub(crate) mod encoder;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
