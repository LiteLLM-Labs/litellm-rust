//! Each subfolder is one provider, discovered automatically at build time (see
//! build.rs) — `register_all` below is generated. No edits here to add one.
//! Networking belongs in `http/llm.rs`, never in a provider.

pub mod router;
pub mod transform;

include!(concat!(env!("OUT_DIR"), "/providers_generated.rs"));
