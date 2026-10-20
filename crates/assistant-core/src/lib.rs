//! Local canonical history: provider sessions are adapters, never the source of truth.
//!
//! This crate never calls an AI service or reads authentication files. It provides
//! persistence, bounded context, and durable task/tool records for future runners.

mod context;
mod providers;
mod store;
mod types;

pub use providers::{connector_specifications, detect_providers};
pub use store::{AssistantError, AssistantResult, AssistantStore};
pub use types::*;
