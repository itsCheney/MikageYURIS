//! # yuris-format
//!
//! Readers for the game's files.
//!
//! | Module | Content |
//! |---|---|
//! | [`ypf`] | YPF archives (offset width and name key inferred) |
//! | [`evb`] | Enigma Virtual Box packages (in an exe or external, e.g. `.mys`); MPL-2.0, from ethornell-rs |
//! | [`vfs`] | The game file system (loose files, archives, save folder, EVB overlay) |
//! | [`ystb`] | YSTB scripts (sections, XOR key, command groups and windows) |
//! | [`yscm`] | YSCM command table (`ysc.ybn`) |
//! | [`yscf`] | YSCF project config (`yscfg.ybn`) |
//! | [`ysvr`] | YSVR variable table (`ysv.ybn`) |
//! | [`yssd`] | YSSD save data (`save\*.sd`) and SNP (Snappy) |
//! | [`yslb`] | YSLB labels (`ysl.ybn`) |
//! | [`ystl`] | YSTL script list (`yst_list.ybn`) |
//!
//! The formats are described in `docs/formats/`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod evb;
pub mod vfs;
pub mod ypf;
pub mod yscf;
pub mod yscm;
pub mod yslb;
pub mod ystb;
pub mod ystl;
pub mod ysvr;
pub mod yssd;

/// Crate version (the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod system_profiles;
pub mod detection;
