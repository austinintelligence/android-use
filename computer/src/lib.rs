#![forbid(unsafe_code)]

pub mod adapter;
mod agent;
pub mod api;
mod apk;
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
pub mod link;
pub use format::{env_notes, render_screen};
pub mod install;
mod journal;
mod marks;
mod present;
mod resolve;
mod routine;
pub mod spec;
pub mod tree;
mod visual;
