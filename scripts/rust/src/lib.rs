#![forbid(unsafe_op_in_unsafe_fn)]

pub mod actions;
pub mod adb;
pub mod app;
pub mod batch;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod device;
pub mod error;
pub mod files;
pub mod helper;
pub mod location;
pub mod media;
pub mod output;
pub mod persistent;
pub mod process;
pub mod protocol;
pub mod selector;
pub mod system;
pub mod tape;
pub mod trace;
pub mod vision;
pub mod web;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;
pub const MAX_PROTOCOL_FRAME: usize = 1024 * 1024;
