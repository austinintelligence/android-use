#![forbid(unsafe_code)]

pub mod adapter;
pub mod api;
#[cfg(test)]
mod api_tests;
pub mod artifact;
pub mod bridge;
pub mod browser;
mod command;
pub mod device;
pub mod engine;
pub mod install;
pub mod visual;
