//! memorph CLI crate — 应用层(交互形态)。
//!
//! 聚集一切交付形态:clap 命令、TUI、HTTP API/Web server、hooks/skills 的
//! HTTP handler。核心领域逻辑在 `memorph` crate。

pub mod api;
pub mod cli;
pub mod hooks;
pub mod server;
pub mod skills;
pub mod tui;
pub mod web;
pub mod web_assets;
