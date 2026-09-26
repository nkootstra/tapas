use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::Path;

use crate::process::RunReport;

/// Environment variable naming the file that receives one JSON line per
/// user-facing command run. Recording is opt-in: with the variable unset or
/// empty this is a no-op and the CLI behaves exactly as before.
pub(crate) const METRICS_PATH_ENV: &str = "TAPAS_COMPACTION_METRICS_PATH";

/// Append a compaction metric for `report` when recording is enabled.
///
/// Failures are deliberately swallowed: telemetry must never change the exit
/// status or output of the wrapped command.
pub(crate) fn record_if_enabled(report: &RunReport) {
    let Ok(path) = std::env::var(METRICS_PATH_ENV) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let _ = record_metric(Path::new(&path), report);
}

fn record_metric(path: &Path, report: &RunReport) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let lock = FileLock::acquire(file.as_raw_fd())?;
    file.write_all(format_metric_line(report).as_bytes())?;
    drop(lock);
    Ok(())
}

fn format_metric_line(report: &RunReport) -> String {
    format!(
        concat!(
            "{{\"command\":\"{}\",\"filter_name\":\"{}\",\"evidence\":\"{}\",",
            "\"raw_bytes\":{},\"displayed_bytes\":{},\"diagnostic_bytes\":{},",
            "\"changed\":{},\"exit_code\":{},\"capture_complete\":{},",
            "\"capture_overflowed\":{}}}\n"
        ),
        escape_json_string(&report.command),
        escape_json_string(report.filter_name),
        evidence_label(report.evidence),
        report.input_bytes,
        report.displayed_bytes,
        report.diagnostic_bytes,
        report.changed,
        report.exit_code,
        report.capture_complete,
        report.capture_overflowed,
    )
}

fn evidence_label(evidence: crate::filters::EvidenceClass) -> &'static str {
    match evidence {
        crate::filters::EvidenceClass::ByteExact => "byte_exact",
        crate::filters::EvidenceClass::FactComplete => "fact_complete",
        crate::filters::EvidenceClass::PotentiallyLossy => "potentially_lossy",
    }
}

fn escape_json_string(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Exclusive advisory lock so concurrent wrapped commands cannot interleave
/// partial lines in the metrics file.
struct FileLock {
    fd: i32,
}

impl FileLock {
    fn acquire(fd: i32) -> io::Result<Self> {
        loop {
            match unsafe { libc::flock(fd, libc::LOCK_EX) } {
                0 => return Ok(Self { fd }),
                -1 => {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                _ => continue,
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.fd, libc::LOCK_UN) };
    }
}
