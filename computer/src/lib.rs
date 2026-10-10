#![forbid(unsafe_code)]

pub mod adapter;
mod agent;
pub mod api;
mod artifact;
pub mod bridge;
mod browser;
pub mod clients;
mod command;
mod compress;
pub mod device;
pub mod emulator;
pub mod engine;
mod format;
pub use format::render_screen;
pub mod install;
mod journal;
mod marks;
mod resolve;
pub mod spec;
mod visual;
