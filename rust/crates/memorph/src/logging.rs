use crate::config;
use chrono::Utc;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub fn info(event: &str, message: impl AsRef<str>) {
    write_line("INFO", event, message.as_ref());
}

pub fn error(event: &str, message: impl AsRef<str>) {
    write_line("ERROR", event, message.as_ref());
}

fn write_line(level: &str, event: &str, message: &str) {
    let Ok(prefs) = config::web_preferences() else {
        return;
    };
    let Ok(dir) = config::memorph_dir().map(|dir| dir.join("logs")) else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    apply_retention(&dir, prefs.logging.retention_days);
    let path = dir.join("memorph.log");
    rotate_if_needed(&path, prefs.logging.max_size_bytes);

    let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let line = format!(
        "{} [{}] {} {}\n",
        timestamp,
        level,
        event,
        message.replace('\n', "\\n")
    );
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn rotate_if_needed(path: &Path, max_size_bytes: u64) {
    if max_size_bytes == 0 {
        return;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() < max_size_bytes {
        return;
    }
    let rotated = path.with_file_name(format!("memorph-{}.log", Utc::now().format("%Y%m%d%H%M%S")));
    let _ = fs::rename(path, rotated);
}

fn apply_retention(dir: &Path, retention_days: Option<u32>) {
    let Some(days) = retention_days else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let Ok(cutoff) = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(days as u64 * 24 * 60 * 60))
        .ok_or(())
    else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("memorph-") || !name.ends_with(".log") {
            continue;
        }
        if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) {
            if modified < cutoff {
                let _ = fs::remove_file(path);
            }
        }
    }
}
