//! A small text log in the data folder (`tessera.log`) for things a user can send along when
//! something goes wrong: failed jobs, panics, a failed ffmpeg download. Never fails the caller.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Past this size the file moves to `tessera.log.1` (one old copy is kept).
const MAX_BYTES: u64 = 512 * 1024;

pub fn path() -> PathBuf {
    crate::settings::data_dir().join("tessera.log")
}

/// `2026-10-22 08:12:03Z` (UTC) for a Unix time.
fn stamp(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil-from-days (Howard Hinnant): days since 1970-01-01 -> y-m-d
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Appends one line. Newlines in `msg` are flattened so one event stays one line.
pub fn write(level: &str, msg: &str) {
    let file = path();
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(&file).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(&file, file.with_extension("log.1"));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let line = format!(
        "{} [{level}] {}\n",
        stamp(now),
        msg.replace(['\r', '\n'], " ")
    );
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&file) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Panics anywhere (also worker threads) end up in the log before the default report.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write("panic", &info.to_string());
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_are_utc_dates() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00Z");
        assert_eq!(stamp(951_782_400), "2000-02-29 00:00:00Z");
        assert_eq!(stamp(1_792_656_723), "2026-10-22 08:12:03Z");
    }
}
