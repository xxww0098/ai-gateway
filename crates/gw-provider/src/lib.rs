//! Upstream provider executors + wire-protocol translation.
//!
//! Provider executors + wire-protocol translation for the five upstreams.
//!
//! OWNERSHIP IS SPLIT — do not write outside your files:
//!   worker `provider-openai`  -> common.rs, openai.rs, codex.rs, usage.rs, streambuf.rs
//!   worker `provider-claude`  -> claude.rs, gemini.rs, vertex.rs
//! `types.rs` and this file are the shared contract: coordinator-owned, ask
//! before changing.

// Ratchet (CONTRACT §2 / §7.4): this crate's public paths are free of `todo!()`
// and stay that way. Two named lints, not a blanket `deny(warnings)`.
#![deny(clippy::todo, clippy::unimplemented)]

pub mod types;

pub mod claude;
pub mod codex;
pub mod common;
pub mod gemini;
pub mod kiro;
pub mod openai;
pub mod streambuf;
pub mod usage;
pub mod vertex;
pub mod xai;

pub use types::*;
