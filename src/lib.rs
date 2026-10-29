//! grh as a library: the binary in `main.rs` is a thin CLI over these modules.
//! Public items here are the surface install, uninstall and status are built on.

pub mod audit;
pub mod catalog;
pub mod dispatch;
pub mod exit;
pub mod extension;
pub mod fetch;
pub mod install;
pub mod manifest;
pub mod ops;
pub mod owner;
pub mod receipt;
pub mod skills;
pub mod status;
pub mod uninstall;
