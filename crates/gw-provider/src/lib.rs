//! Upstream **route planning** + wire-protocol usage parsing.
//!
//! # This crate does not send inference HTTP
//!
//! It used to: five executors, five `reqwest` pools, five copies of "read the
//! response, look for usage, turn a 4xx into an error". `gw-relay` is the
//! byte-level pass-through engine, and two engines is one too many — the
//! provider copies dropped headers on non-2xx, collapsed multi-valued
//! `set-cookie`, and reported a mid-stream failure as a clean EOF.
//!
//! What is left here is the knowledge `gw-relay` deliberately does not have:
//! which endpoint an account exposes, which credential signs it, and what the
//! upstream's usage envelope looks like. That is [`RoutePlanner::plan`], and it
//! returns a [`RoutePlan`] — a plain value, no sockets.
//!
//! The one HTTP that stays is **credential refresh** ([`RoutePlanner::refresh`]):
//! it talks to an identity provider's token endpoint, carries no tenant
//! payload, and its response never reaches a client. It lives in `oauth.rs`,
//! which is the only module in this crate allowed to send.

// Ratchet (CONTRACT §2 / §7.4): this crate's public paths are free of `todo!()`
// and stay that way. Two named lints, not a blanket `deny(warnings)`.
#![deny(clippy::todo, clippy::unimplemented)]

mod oauth;
pub mod route;
pub mod types;

pub mod claude;
pub mod codex;
pub mod common;
pub mod gemini;
pub mod kiro;
pub mod local_oauth;
pub mod openai;
pub mod usage;
pub mod vertex;
pub mod xai;

pub use route::{RoutePlan, RoutePlanner};
pub use types::*;
