//! The concrete backends.
//!
//! Each one's only job is to speak its provider's wire format and hand the
//! raw result to [`crate::guard::finalize`]. Caching, throttling and
//! budgeting are **not** here — they are decorators (§10.5), so that a new
//! backend gets all three for free and cannot accidentally implement one of
//! them slightly differently.

mod gemini;
mod http;
mod mock;
mod ollama;
mod replay;

pub use gemini::{GEMINI_API_KEY_VAR, GeminiProvider};
pub use mock::{MockProvider, MockStep};
pub use ollama::{OLLAMA_API_KEY_VAR, OllamaProvider, shared as shared_ollama, validate_tag};
pub use replay::ReplayProvider;
