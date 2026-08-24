#![deny(unsafe_code)]

pub mod cli;
pub mod config;
#[cfg(any(not(windows), test))]
pub mod console;
pub mod database;
pub mod error;
pub mod filesystem;
pub mod image;
pub mod isolation;
pub mod network;
pub mod platform;
pub mod runtime;
#[cfg(any(not(windows), test, feature = "gui"))]
pub mod service;
pub mod state;
pub mod storage;

pub use config::RuntimeConfig;
pub use error::{Result, RuntimeError};
pub use state::StateLayout;
