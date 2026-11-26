//! 出错记录：`%LOCALAPPDATA%\OneZip\onezip.log`，只记出错和少量关键事件。
//! 超过 512 KB 时把旧的改名为 `onezip.old.log`，只留这两份。设置里的"报告问题"会附上最近几条。

use onezip_i18n::t;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 512 << 10;

fn dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("OneZip"))
}

pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join("onezip.log"))
}

/// 记一条出错。写日志本身失败时什么也不做（不能因为日志让程序出错）。
pub fn error(msg: &str) {
    write(t!("错误"), msg);
}

/// 记一条值得留意的事件（如开始更新）。
pub fn info(msg: &str) {
    write(t!("信息"), msg);
}

fn write(level: &str, msg: &str) {
    let Some(path) = path() else { return };
    if let Some(d) = path.parent() {
        let _ = fs::create_dir_all(d);
    }
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(&path, path.with_file_name("onezip.old.log"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} [{level}] {}", now_text(), msg.replace('\n', " / "));
    }
}

/// 最近的 `n` 行。
pub fn recent(n: usize) -> Vec<String> {
    let Some(text) = path().and_then(|p| fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|s| s.to_string()).collect()
}

/// 现在的 UTC 时间，如 `2026-11-25 10:21:03Z`。
fn now_text() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    utc_text(secs)
}

fn utc_text(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // 公历日期（Howard Hinnant 的 civil_from_days）
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

/// 程序崩溃（panic）时也记一条。
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        error(&t!("程序崩溃：{}", info));
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc() {
        assert_eq!(super::utc_text(0), "1970-01-01 00:00:00Z");
        assert_eq!(super::utc_text(1_795_634_536), "2026-11-25 19:22:16Z");
    }
}
