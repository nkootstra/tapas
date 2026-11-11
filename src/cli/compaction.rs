use std::ffi::OsStr;
use std::fs::{DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::process::RunReport;

/// Environment variable naming the file that receives one JSON line per
/// user-facing command run. When set to a non-empty value it always wins:
/// every run records there, hook-driven or not.
pub(crate) const METRICS_PATH_ENV: &str = "TAPAS_COMPACTION_METRICS_PATH";

/// Opt-out for the default recording of hook-driven runs: `0`, `false`, `no`
/// or `off` turns it off. It does not affect an explicit metrics path.
pub(crate) const METRICS_ENV: &str = "TAPAS_COMPACTION_METRICS";

/// Default metrics file, relative to `$HOME`, for runs launched by an agent
/// hook rewrite. The file stays local; nothing reads it except the user.
const DEFAULT_METRICS_FILE: &str = ".tapas/metrics.jsonl";

/// Append a compaction metric for `report` when recording is enabled.
///
/// Failures are deliberately swallowed: telemetry must never change the exit
/// status or output of the wrapped command.
pub(crate) fn record_if_enabled(report: &RunReport) {
    let hook = launched_by_hook_rewrite(
        std::env::args_os().next().as_deref(),
        std::env::current_exe().ok().as_deref(),
    );
    let Some(path) = resolve_metrics_path(
        std::env::var_os(METRICS_PATH_ENV).as_deref(),
        std::env::var_os(METRICS_ENV).as_deref(),
        std::env::var_os("HOME").as_deref(),
        hook,
    ) else {
        return;
    };
    let _ = record_metric(&path, report, hook);
}

/// Whether this process was started by an agent hook rewrite.
///
/// Every hook target (Claude Code, Codex, OpenCode) rewrites a command to the
/// shell-quoted absolute path of the tapas executable followed by the original
/// command, so the shell passes that absolute path as `argv[0]`. A command
/// typed by a person or an agent says `tapas`, which the shell passes as the
/// bare name. Reading `argv[0]` needs no change to the rewritten command, so
/// a permission classifier reviewing that command sees nothing new.
fn launched_by_hook_rewrite(argv0: Option<&OsStr>, current_exe: Option<&Path>) -> bool {
    let (Some(argv0), Some(current_exe)) = (argv0, current_exe) else {
        return false;
    };
    let argv0 = Path::new(argv0);
    argv0.is_absolute() && argv0 == current_exe
}

fn resolve_metrics_path(
    explicit: Option<&OsStr>,
    opt_out: Option<&OsStr>,
    home: Option<&OsStr>,
    hook: bool,
) -> Option<PathBuf> {
    if let Some(explicit) = explicit.filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let opted_out = opt_out
        .is_some_and(|value| matches!(value.as_encoded_bytes(), b"0" | b"false" | b"no" | b"off"));
    if !hook || opted_out {
        return None;
    }
    default_metrics_path(home?)
}

fn default_metrics_path(home: &OsStr) -> Option<PathBuf> {
    let home = Path::new(home);
    home.is_absolute().then(|| home.join(DEFAULT_METRICS_FILE))
}

fn record_metric(path: &Path, report: &RunReport, hook: bool) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    let lock = FileLock::acquire(file.as_raw_fd())?;
    file.write_all(format_metric_line(report, hook).as_bytes())?;
    drop(lock);
    Ok(())
}

fn format_metric_line(report: &RunReport, hook: bool) -> String {
    format!(
        concat!(
            "{{\"command\":\"{}\",\"filter_name\":\"{}\",\"evidence\":\"{}\",",
            "\"raw_bytes\":{},\"displayed_bytes\":{},\"diagnostic_bytes\":{},",
            "\"lines_dropped\":{},",
            "\"changed\":{},\"exit_code\":{},\"capture_complete\":{},",
            "\"capture_overflowed\":{},\"hook\":{}}}\n"
        ),
        escape_json_string(&report.command),
        escape_json_string(report.filter_name),
        evidence_label(report.evidence),
        report.input_bytes,
        report.displayed_bytes,
        report.diagnostic_bytes,
        report.lines_dropped,
        report.changed,
        report.exit_code,
        report.capture_complete,
        report.capture_overflowed,
        hook,
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

#[cfg(test)]
mod tests {
    use super::{default_metrics_path, launched_by_hook_rewrite, resolve_metrics_path};
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    const EXE: &str = "/opt/tapas/bin/tapas";

    fn resolve(
        explicit: Option<&str>,
        opt_out: Option<&str>,
        home: Option<&str>,
        hook: bool,
    ) -> Option<PathBuf> {
        resolve_metrics_path(
            explicit.map(OsStr::new),
            opt_out.map(OsStr::new),
            home.map(OsStr::new),
            hook,
        )
    }

    #[test]
    fn hook_runs_default_to_a_file_under_home() {
        assert_eq!(
            resolve(None, None, Some("/home/u"), true),
            Some(PathBuf::from("/home/u/.tapas/metrics.jsonl"))
        );
        assert_eq!(
            default_metrics_path(OsStr::new("/home/u")),
            Some(PathBuf::from("/home/u/.tapas/metrics.jsonl"))
        );
    }

    #[test]
    fn direct_runs_record_nothing_by_default() {
        assert_eq!(resolve(None, None, Some("/home/u"), false), None);
        assert_eq!(resolve(Some(""), None, Some("/home/u"), false), None);
    }

    #[test]
    fn opt_out_disables_the_default() {
        for value in ["0", "false", "no", "off"] {
            assert_eq!(
                resolve(None, Some(value), Some("/home/u"), true),
                None,
                "{value}"
            );
        }
        assert_eq!(
            resolve(None, Some("1"), Some("/home/u"), true),
            Some(PathBuf::from("/home/u/.tapas/metrics.jsonl"))
        );
    }

    #[test]
    fn explicit_path_wins_over_default_and_opt_out() {
        for hook in [true, false] {
            assert_eq!(
                resolve(Some("/tmp/m.jsonl"), Some("0"), Some("/home/u"), hook),
                Some(PathBuf::from("/tmp/m.jsonl"))
            );
        }
        // An empty explicit path is the same as unset.
        assert_eq!(
            resolve(Some(""), None, Some("/home/u"), true),
            Some(PathBuf::from("/home/u/.tapas/metrics.jsonl"))
        );
    }

    #[test]
    fn unusable_home_disables_the_default() {
        assert_eq!(resolve(None, None, None, true), None);
        assert_eq!(resolve(None, None, Some(""), true), None);
        assert_eq!(resolve(None, None, Some("relative/home"), true), None);
    }

    #[test]
    fn hook_rewrite_is_the_absolute_executable_path_as_argv0() {
        let exe = Path::new(EXE);
        assert!(launched_by_hook_rewrite(Some(OsStr::new(EXE)), Some(exe)));
        assert!(!launched_by_hook_rewrite(
            Some(OsStr::new("tapas")),
            Some(exe)
        ));
        assert!(!launched_by_hook_rewrite(
            Some(OsStr::new("bin/tapas")),
            Some(Path::new("bin/tapas"))
        ));
        assert!(!launched_by_hook_rewrite(
            Some(OsStr::new("/usr/local/bin/tapas")),
            Some(exe)
        ));
        assert!(!launched_by_hook_rewrite(None, Some(exe)));
        assert!(!launched_by_hook_rewrite(Some(OsStr::new(EXE)), None));
    }
}
