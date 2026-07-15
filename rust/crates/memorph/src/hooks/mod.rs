//! Runtime hook integration for memorph.
//!
//! Hook ingestion, provider event normalization, runtime session tracking, hook
//! installation, health checks, and diagnostics live behind this boundary and
//! are wired into the existing
//! CLI/TUI/Web/Desktop entry points.

pub mod adapters;
pub mod augmentation;
pub mod bridge;
pub mod capabilities;
pub mod config_formats;
pub mod contract;
pub mod correlation;
pub mod diagnostics;
pub mod doctor;
pub mod extension_file_hook;
pub mod health;
pub mod identity;
pub mod json_settings_hook;
pub mod lifecycle;
pub mod model;
pub mod normalizer;
pub mod operations;
pub mod profiles;
pub mod protocol;
pub mod registry;
pub mod runtime;
pub mod server;
pub mod shared;
pub mod store;
pub mod strategies;

#[cfg(test)]
pub(crate) mod test_support;
