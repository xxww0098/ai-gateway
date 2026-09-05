//! Shared OAuth **hop planning**. No sockets.
//!
//! `dsh-plugin-oauth-subs` and `gw-provider` both need the same vendor
//! fingerprints: cache ids, identity headers, and the body fields each
//! upstream 400s on. This crate is that knowledge as values.
//!
//! Inference HTTP stays in `gw-relay`. Credential refresh stays in
//! `gw-provider::oauth`. Authorization / `Credential` never appear here —
//! a hop header map that carries a bearer is a second credential path.
//!
//! Families do not import each other. Cache helpers for Codex stay in
//! [`codex`]; Grok's stay in [`grok`]; Kiro's stay in [`kiro`].

#![deny(clippy::todo, clippy::unimplemented)]

pub mod codex;
pub mod family;
pub mod grok;
pub mod id;
pub mod kiro;
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
