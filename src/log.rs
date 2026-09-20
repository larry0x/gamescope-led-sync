//! Tiny stderr logger matching the Python implementation's format:
//! `HH:MM:SS LEVEL message`, timestamps in UTC.

use std::time::{SystemTime, UNIX_EPOCH};

fn ts() -> String {
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600 % 24, s / 60 % 60, s % 60)
}

pub fn info(msg: impl AsRef<str>) {
    eprintln!("{} INFO {}", ts(), msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    eprintln!("{} WARNING {}", ts(), msg.as_ref());
}
