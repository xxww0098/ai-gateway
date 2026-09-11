//! Vendor OAuth families for AI-GateWay.
//!
//! One folder per family. Cache helpers must not be imported across families;
//! [`rewrite`] only dispatches.

#![deny(clippy::todo, clippy::unimplemented)]

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod error;
pub mod family;
pub mod form;
pub mod glm;
pub mod grok;
pub mod jwt;
pub mod kimi;
pub mod kiro;
pub mod login;
pub mod ollama;
pub mod pkce;
pub mod plan;
pub mod rewrite;
pub mod session;

pub use error::Error;
pub use family::{Family, Hop};
pub use login::{StartInput, start};
pub use jwt::{decode_payload, email_and_account};
pub use pkce::{Pkce, create_pkce, random_hex, random_token};
pub use plan::format_plan_label;
pub use rewrite::{HeaderExtra, cache_headers, rewrite_body};
pub use session::{CacheRewrite, FlowKind, Session, StartOutcome};
