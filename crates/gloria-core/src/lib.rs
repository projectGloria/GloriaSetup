//! GloriaSetup shared logic. No UI and no Win32 calls that need a desktop session.

pub mod archive;
pub mod cli;
pub mod exit;
pub mod install_manifest;
pub mod journal;
pub mod legacy;
pub mod manifest;
pub mod paths;
pub mod payload;
pub mod registry;
pub mod res;
pub mod version;

/// The GloriaSetup version (stub and packer share it).
pub const GLORIA_SETUP_VERSION: &str = env!("CARGO_PKG_VERSION");
