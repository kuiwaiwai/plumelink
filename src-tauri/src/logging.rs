use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_DAILY_BYTES: u64 = 5 * 1024 * 1024;
const MAX_RETAINED_DAYS: usize = 7;

struct ActiveFile {
    day: i64,
    bytes: u64,
    file: File,
}

struct Logger {
    dir: PathBuf,
    state: Mutex<Option<ActiveFile>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init(dir: PathBuf) -> Result<(), String> {
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建日志目录：{e}"))?;
    prune_old_logs(&dir, MAX_RETAINED_DAYS);
    let _ = LOGGER.set(Logger {
        dir,
        state: Mutex::new(None),
    });
    Ok(())
}

pub fn dir() -> Option<PathBuf> {
    LOGGER.get().map(|logger| logger.dir.clone())
}

pub fn info(scope: &str, message: &str) {
    write_record("INFO", scope, message);
}

pub fn warn(scope: &str, message: &str) {
    write_record("WARN", scope, message);
}

pub fn error(scope: &str, message: &str) {
    write_record("ERROR", scope, message);
}

fn write_record(level: &str, scope: &str, message: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let (day, timestamp) = format_utc_now(SystemTime::now());
    let clean_scope = sanitize_inline(scope);
    let clean_msg = sanitize_inline(message);
    let line = format!("[{timestamp}] [{level}] [{clean_scope}] {clean_msg}\n");
    let mut guard = logger.state.lock().unwrap();
    let _ = write_to_dir(&logger.dir, &mut guard, day, line.as_bytes());
}

fn write_to_dir(
    dir: &Path,
    slot: &mut Option<ActiveFile>,
    day: i64,
    line: &[u8],
) -> std::io::Result<()> {
    if slot.as_ref().is_none_or(|active| active.day != day) {
        *slot = None;
        prune_old_logs(dir, MAX_RETAINED_DAYS.saturating_sub(1));
        let (year, month, dom) = civil_from_days(day);
        let path = dir.join(format!("plumelink-{year:04}-{month:02}-{dom:02}.log"));
        let existing_len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        *slot = Some(ActiveFile {
            day,
            bytes: existing_len,
            file,
        });
    }
    if let Some(active) = slot.as_mut() {
        let next = active.bytes.saturating_add(line.len() as u64);
        if next > MAX_DAILY_BYTES {
            return Ok(());
        }
        active.file.write_all(line)?;
        active.file.flush()?;
        active.bytes = next;
    }
    Ok(())
}

fn sanitize_inline(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\r' | '\n' | '\t' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect()
}

fn is_daily_log_name(name: &str) -> bool {
    name.starts_with("plumelink-")
        && name.ends_with(".log")
        && name.len() == "plumelink-YYYY-MM-DD.log".len()
}

fn prune_old_logs(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_daily_log_name(&name) {
                Some((name, entry.path()))
            } else {
                None
            }
        })
        .collect();
    if logs.len() <= keep {
        return;
    }
    logs.sort_by(|a, b| a.0.cmp(&b.0));
    let remove_count = logs.len().saturating_sub(keep);
    for (_, path) in logs.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

fn format_utc_now(now: SystemTime) -> (i64, String) {
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();
    let day = (total_secs / 86_400) as i64;
    let rem = total_secs % 86_400;
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;
    let (year, month, dom) = civil_from_days(day);
    (
        day,
        format!("{year:04}-{month:02}-{dom:02} {hour:02}:{minute:02}:{second:02}.{millis:03}Z"),
    )
}

// Howard Hinnant's branchless civil_from_days algorithm (Unix epoch 1970-01-01 = day 0).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    (year, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_calendar_and_utc_formatting_match_epoch_and_leap_day() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        let (_, stamp) = format_utc_now(UNIX_EPOCH + std::time::Duration::from_millis(3_661_042));
        assert_eq!(stamp, "1970-01-01 01:01:01.042Z");
    }

    #[test]
    fn rolling_log_prunes_old_files_and_caps_daily_size() {
        let dir = std::env::temp_dir().join(format!(
            "plumelink-log-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let mut slot = None;
        for day in 20_000..20_010 {
            write_to_dir(&dir, &mut slot, day, b"event\n").unwrap();
        }
        drop(slot);
        let remaining: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| is_daily_log_name(&e.file_name().to_string_lossy()))
            .collect();
        assert_eq!(remaining.len(), MAX_RETAINED_DAYS);

        let mut big_slot = Some(ActiveFile {
            day: 20_011,
            bytes: MAX_DAILY_BYTES - 4,
            file: OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("plumelink-cap.log"))
                .unwrap(),
        });
        write_to_dir(&dir, &mut big_slot, 20_011, b"overflow\n").unwrap();
        assert_eq!(
            fs::metadata(dir.join("plumelink-cap.log")).unwrap().len(),
            0
        );
        drop(big_slot);
        fs::remove_dir_all(dir).unwrap();
    }
}
