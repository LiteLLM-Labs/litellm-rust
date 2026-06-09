//! Canonical intermediate representation (IR). Every wire protocol parses into
//! and renders out of these types, so converting between N protocols needs N
//! codecs instead of N×N point-to-point translators. The shape mirrors
//! Anthropic content blocks because they are the most expressive of the four
//! protocols (text / tool calls / tool results / thinking are all blocks).

mod content;
mod request;
mod response;
mod stream;

pub use content::*;
pub use request::*;
pub use response::*;
pub use stream::*;
