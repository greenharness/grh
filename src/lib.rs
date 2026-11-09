//! grh as a library: the binary in `main.rs` is a thin CLI over these modules.
//! Public items here are the surface install, uninstall and status are built on.
//! `pins` is the exception: maintainer tooling used by the `pins` example and
//! the tests, not by any grh command.

pub mod audit;
pub mod catalog;
pub mod checklist;
pub mod dispatch;
pub mod exit;
pub mod extension;
pub mod fetch;
pub mod install;
pub mod manifest;
pub mod ops;
pub mod owner;
pub mod pins;
pub mod receipt;
pub mod skills;
pub mod status;
pub mod uninstall;
