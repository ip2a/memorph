//! Provider-specific hook adapters.
//!
//! Planned responsibility:
//! - Keep provider payload parsing isolated per provider.
//! - Export adapters used by `hooks::normalizer`.
//! - Start with the highest-value providers, then add more without changing runtime state code.

pub mod antigravity;
pub mod claude;
pub mod cline;
pub mod codebuddy;
pub mod codex;
pub mod codybuddycn;
pub mod copilot;
pub mod cursor;
pub mod droid;
pub mod gemini;
pub mod generic;
pub mod hermes;
pub mod kimi;
pub mod kiro;
pub mod omp;
pub mod opencode;
pub mod pi;
pub mod qoder;
pub mod qwen;
pub mod stepfun;
pub mod trae;
pub mod trae_gui;
pub mod traecn;
pub mod workbuddy;
