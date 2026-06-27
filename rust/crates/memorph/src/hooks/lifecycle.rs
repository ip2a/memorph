//! Runtime hook lifecycle maintenance.
//!
//! This is the process/session cleanup layer inspired by CodeIsland's runtime
//! cleanup loop. It keeps persisted hook sessions from accumulating stale active
//! state when providers exit without emitting a final event.

use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::hooks::protocol::HookProcessInfo;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCleanupOptions {
    pub idle_after_seconds: i64,
    pub orphan_after_seconds: i64,
}

impl Default for RuntimeCleanupOptions {
    fn default() -> Self {
        Self {
            idle_after_seconds: 30 * 60,
            orphan_after_seconds: 60 * 60,
        }
    }
}

impl RuntimeCleanupOptions {
    pub fn idle_after(&self) -> Duration {
        Duration::seconds(self.idle_after_seconds.max(1))
    }

    pub fn orphan_after(&self) -> Duration {
        Duration::seconds(self.orphan_after_seconds.max(1))
    }
}

pub fn pid_is_alive(pid: u32) -> bool {
    pid_is_alive_with_start_time(pid, None)
}

pub fn pid_is_alive_with_start_time(pid: u32, expected_start_time: Option<&str>) -> bool {
    if pid == 0 {
        return false;
    }
    let alive = pid_exists(pid);
    if !alive {
        return false;
    }
    if let Some(expected_start_time) = expected_start_time.filter(|value| !value.trim().is_empty())
    {
        return process_start_time(pid).as_deref() == Some(expected_start_time);
    }
    true
}

fn pid_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        return std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
    }

    #[cfg(windows)]
    {
        return std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.split_whitespace().any(|part| part == pid.to_string()))
            })
            .unwrap_or(false);
    }

    #[allow(unreachable_code)]
    false
}

pub fn process_start_time(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let output = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn process_parent_pid(pid: u32) -> Option<u32> {
    let value = ps_value(pid, "ppid=")?;
    value.trim().parse().ok()
}

pub fn process_command(pid: u32) -> Option<String> {
    ps_value(pid, "comm=").filter(|value| !value.trim().is_empty())
}

pub fn process_tty(pid: u32) -> Option<String> {
    normalize_tty(&ps_value(pid, "tty=")?)
}

pub fn process_ancestry(starting_pid: u32, max_depth: usize) -> Vec<HookProcessInfo> {
    let mut result = Vec::new();
    let mut pid = starting_pid;
    for _ in 0..max_depth {
        if pid == 0 {
            break;
        }
        let parent_pid = process_parent_pid(pid);
        result.push(HookProcessInfo {
            pid,
            parent_pid,
            command: process_command(pid),
            start_time: process_start_time(pid),
        });
        let Some(parent_pid) = parent_pid else {
            break;
        };
        if parent_pid == 0 || parent_pid == pid {
            break;
        }
        pid = parent_pid;
    }
    result
}

fn normalize_tty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || matches!(value, "?" | "??" | "-") {
        return None;
    }
    if value.starts_with("/dev/") {
        Some(value.to_string())
    } else {
        Some(format!("/dev/{value}"))
    }
}

fn ps_value(pid: u32, column: &str) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let output = std::process::Command::new("ps")
        .args(["-o", column, "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(pid_is_alive(std::process::id()));
    }

    #[test]
    fn current_process_start_time_guards_pid_reuse() {
        let pid = std::process::id();
        let token = process_start_time(pid);
        if let Some(token) = token {
            assert!(pid_is_alive_with_start_time(pid, Some(&token)));
            assert!(!pid_is_alive_with_start_time(
                pid,
                Some("definitely-not-this-process")
            ));
        }
    }

    #[test]
    fn process_ancestry_includes_current_process() {
        let ancestry = process_ancestry(std::process::id(), 3);
        assert!(!ancestry.is_empty());
        assert_eq!(ancestry[0].pid, std::process::id());
    }

    #[test]
    fn normalize_tty_filters_empty_and_expands_device_paths() {
        assert_eq!(normalize_tty("ttys001").as_deref(), Some("/dev/ttys001"));
        assert_eq!(
            normalize_tty("/dev/ttys002").as_deref(),
            Some("/dev/ttys002")
        );
        assert_eq!(normalize_tty("?"), None);
        assert_eq!(normalize_tty("??"), None);
        assert_eq!(normalize_tty("-"), None);
        assert_eq!(normalize_tty(" "), None);
    }

    #[test]
    fn default_cleanup_options_are_positive() {
        let options = RuntimeCleanupOptions::default();
        assert!(options.idle_after_seconds > 0);
        assert!(options.orphan_after_seconds > options.idle_after_seconds);
    }
}
