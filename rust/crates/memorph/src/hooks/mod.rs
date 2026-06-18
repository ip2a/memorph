//! Runtime hook integration for memorph.
//!
//! Hook ingestion, provider event normalization, runtime session tracking, hook
//! installation, health checks, policy decisions, pending user decisions, and
//! diagnostics live behind this boundary and are wired into the existing
//! CLI/TUI/Web/Desktop entry points.

pub mod adapters;
pub mod bridge;
pub mod correlation;
pub mod diagnostics;
pub mod doctor;
pub mod health;
pub mod identity;
pub mod installer;
pub mod lifecycle;
pub mod model;
pub mod normalizer;
pub mod policy;
pub mod profiles;
pub mod protocol;
pub mod runtime;
pub mod server;
pub mod store;
pub mod visibility;
