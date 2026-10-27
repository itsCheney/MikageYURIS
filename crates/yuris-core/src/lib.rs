//! # yuris-core
//!
//! Shared basics for the other crates: the error type, a byte reader, the
//! hashes (murmur2, a fast hasher) and version profiles. No engine logic
//! lives here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bytes;
pub mod error;
pub mod hash;
pub mod version;

pub use bytes::{xor_cyclic_skip, Reader};
pub use error::{Error, Result};
pub use hash::{murmur2, FastBuildHasher, FastHashMap, FastHasher};
pub use version::{CharEncoding, VersionProfile};

/// Crate version (the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod system_vars;
pub use system_vars::SystemVarMap;
