//! yuris-tools
//!
//! The `yuris` command line tool (`src/main.rs`): archive and script
//! inspection and the headless `boot` runner. See the README.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Crate version (the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
