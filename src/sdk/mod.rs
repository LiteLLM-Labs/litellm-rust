//! SDK layer — everything that talks to upstream LLM providers and decides
//! which deployment a request maps to. Must not depend on `proxy/` so it can
//! ship standalone.

pub mod providers;
pub mod router;
