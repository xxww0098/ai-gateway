//! Three-rank identity algebra (`user` / `admin` / `super_admin`).
//!
//! Implementation lives in [`role_core`] so another project can copy that
//! single file. This crate has no dependencies.

#![deny(clippy::todo, clippy::unimplemented)]

pub mod role_core;
pub use role_core::*;
