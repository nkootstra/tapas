use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::Path;

use crate::process::RunReport;

pub(crate) fn record_if_enabled(report: &RunReport) {
    let Ok(path) = std::env::var("TAPAS_COMPACTION_METRICS_PATH") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let _ = record_compaction_metric(Path::new(&path), report);
}

fn record_compaction_metric(path: &Path, report: &RunReport) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let lock = FileLock::acquire(file.as_raw_fd())?;
    let line = format_compaction_metric_line(report);
    file.write_all(line.as_bytes())?;
    drop(lock);
    Ok(())
}

fn format_compaction_metric_line(report: &RunReport) -> String {
    let command = escape_json_string(&report.command);
    format!(
        "{{\"command\":\"{command}\",\"filter_name\":\"{filter}\",\"evidence\":\"{evidence}\",\"raw_bytes\":{raw_bytes},\"displayed_bytes\":{displayed_bytes},\"diagnostic_bytes\":{diagnostic_bytes},\"changed\":{changed},\"exit_code\":{exit_code},\"capture_complete\":{capture_complete},\"capture_overflowed\":{capture_overflowed}}}\n",
        filter = report.filter_name,
        evidence = evidence_label(report.evidence),
        raw_bytes = report.input_bytes,
        displayed_bytes = report.displayed_bytes,
        diagnostic_bytes = report.diagnostic_bytes,
        changed = report.changed,
        exit_code = report.exit_code,
        capture_complete = report.capture_complete,
        capture_overflowed = report.capture_overflowed,
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
