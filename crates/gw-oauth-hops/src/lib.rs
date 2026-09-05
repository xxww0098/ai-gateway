//! Shared OAuth **hop planning**. No sockets.
//!
//! Replicates `dsh-plugin-oauth-subs` family hops as values: identity
//! headers, cache ids, and the body fields each upstream 400s on.
//!
//! Inference HTTP stays in `gw-relay`. Credential refresh stays in
//! `gw-provider::oauth`. Authorization / `Credential` never appear here —
//! a hop header map that carries a bearer is a second credential path.
//!
//! Families do not import each other.

#![deny(clippy::todo, clippy::unimplemented)]

pub mod antigravity;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod family;
pub mod glm;
pub mod grok;
pub mod id;
pub mod kimi;
pub mod kiro;
pub mod ollama;
pub mod opencode;
pub mod pin;
pub mod rewrite;

pub use family::Family;
pub use pin::{PinResult, PrefixPins};
pub use rewrite::{HopInput, HopRewrite};

/// Insert `src` into `dst`. Later keys in `src` win.
pub fn merge_headers(dst: &mut http::HeaderMap, src: http::HeaderMap) {
    for (name, value) in src {
        if let Some(name) = name {
            dst.insert(name, value);
        }
    }
}

#[cfg(test)]
mod tests;
