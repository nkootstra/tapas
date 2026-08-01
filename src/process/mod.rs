use std::borrow::Cow;
use std::ffi::OsString;
use std::io::{self, Write};

mod capture;
pub mod invocation;
mod unix;

use crate::filters::EvidenceClass;
use capture::CaptureMode;
use invocation::{StreamDecision, classify, classify_stream, is_raw_curl, requests_exact_output};

pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const INCOMPLETE_OUTPUT_DIAGNOSTIC: &[u8] =
    b"(tapas: output incomplete; descendants kept stdout/stderr open after child exit)\n";

#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    pub raw: bool,
    pub explain: bool,
}

#[derive(Debug)]
pub struct RunReport {
    pub exit_code: i32,
    pub input_bytes: usize,
    pub displayed_bytes: usize,
    pub diagnostic_bytes: usize,
    pub filter_name: &'static str,
    pub evidence: EvidenceClass,
    pub capture_complete: bool,
    pub capture_overflowed: bool,
}

pub fn run(
    argv: &[OsString],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    options: RunOptions,
) -> io::Result<RunReport> {
    let invocation = classify(argv);
    let logical = invocation.logical_argv;
    let lossless = crate::environment::flag_on("TAPAS_LOSSLESS");
    let stream = classify_stream(logical);
    let unfiltered = options.raw
        || lossless
        || invocation.passthrough_reason.is_some()
        || stream != StreamDecision::Capture
        || is_raw_curl(logical);

    if unfiltered && unix::outputs_are_tty() {
        let report = RunReport {
            exit_code: capture::run_inherited(argv)?,
            input_bytes: 0,
            displayed_bytes: 0,
            diagnostic_bytes: 0,
            filter_name: "passthrough",
            evidence: EvidenceClass::ByteExact,
            capture_complete: true,
            capture_overflowed: false,
        };
        return return_report(report, stderr, options.explain);
    }

    let mode = if unfiltered {
        CaptureMode::Passthrough
    } else {
        CaptureMode::Buffered {
            limit: MAX_OUTPUT_BYTES,
        }
    };
    let force_c_locale =
        !unfiltered && command_is(logical, b"ls") && !requests_exact_output(logical);
    let captured = capture::run_captured(argv, mode, force_c_locale, stdout, stderr)?;

    if captured.streamed {
        let diagnostic_bytes = write_incomplete_diagnostic(captured.incomplete, stderr)?;
        let report = RunReport {
            exit_code: captured.exit_code,
            input_bytes: captured.input_bytes,
            displayed_bytes: captured.input_bytes + diagnostic_bytes,
            diagnostic_bytes,
            filter_name: "passthrough",
            evidence: EvidenceClass::ByteExact,
            capture_complete: !captured.incomplete,
            capture_overflowed: captured.overflowed,
        };
        return return_report(report, stderr, options.explain);
    }

    if captured.incomplete {
        stdout.write_all(&captured.stdout)?;
        stderr.write_all(&captured.stderr)?;
        let diagnostic_bytes = write_incomplete_diagnostic(true, stderr)?;
        let report = RunReport {
            exit_code: captured.exit_code,
            input_bytes: captured.input_bytes,
            displayed_bytes: captured.input_bytes + diagnostic_bytes,
            diagnostic_bytes,
            filter_name: "passthrough",
            evidence: EvidenceClass::ByteExact,
            capture_complete: false,
            capture_overflowed: false,
        };
        return return_report(report, stderr, options.explain);
    }

    // A transparent runner's package-install prelude belongs to the runner,
    // not the inner command's formatter.
    let has_runner_prelude = crate::filters::build::has_package_prelude(&captured.stdout)
        || crate::filters::build::has_package_prelude(&captured.stderr);
    let transparent_runner = !std::ptr::eq(logical, argv);
    let filter_argv = if transparent_runner && has_runner_prelude {
        argv
    } else {
        logical
    };
    let filtered = filter_captured_output(filter_argv, &captured, lossless);
    let changed =
        filtered.stdout.as_ref() != captured.stdout || filtered.stderr.as_ref() != captured.stderr;
    let failure_fell_open =
        captured.exit_code != 0 && changed && filtered.evidence == EvidenceClass::PotentiallyLossy;
    let visible_stdout = if failure_fell_open {
        captured.stdout.as_slice()
    } else {
        filtered.stdout.as_ref()
    };
    let visible_stderr = if failure_fell_open {
        captured.stderr.as_slice()
    } else {
        filtered.stderr.as_ref()
    };
    let (filter_name, evidence) = if failure_fell_open {
        ("passthrough", EvidenceClass::ByteExact)
    } else {
        (filtered.filter_name, filtered.evidence)
    };

    let diagnostic_bytes = if visible_stdout.is_empty() && visible_stderr.is_empty() {
        let hint = no_output_hint(argv, captured.exit_code);
        stdout.write_all(&hint)?;
        hint.len()
    } else {
        stdout.write_all(visible_stdout)?;
        stderr.write_all(visible_stderr)?;
        0
    };
    let report = RunReport {
        exit_code: captured.exit_code,
        input_bytes: captured.input_bytes,
        displayed_bytes: visible_stdout.len() + visible_stderr.len() + diagnostic_bytes,
        diagnostic_bytes,
        filter_name,
        evidence,
        capture_complete: true,
        capture_overflowed: false,
    };
    return_report(report, stderr, options.explain)
}

struct FilteredStreams<'a> {
    stdout: Cow<'a, [u8]>,
    stderr: Cow<'a, [u8]>,
    filter_name: &'static str,
    evidence: EvidenceClass,
}

fn filter_captured_output<'a>(
    argv: &[OsString],
    captured: &'a capture::CapturedOutput,
    lossless: bool,
) -> FilteredStreams<'a> {
    if requests_exact_output(argv) {
        return passthrough_streams(captured);
    }

    let argv_bytes: Vec<&[u8]> = argv
        .iter()
        .map(|argument| argument.as_encoded_bytes())
        .collect();
    if command_is(argv, b"git")
        && argv.len() > 1
        && let Ok(output) = crate::filters::git::dispatch_streams_argv(
            &argv_bytes,
            &captured.stdout,
            &captured.stderr,
            captured.exit_code,
            lossless,
        )
    {
        let unchanged = output.stdout == captured.stdout && output.stderr == captured.stderr;
        return FilteredStreams {
            stdout: Cow::Owned(output.stdout),
            stderr: Cow::Owned(output.stderr),
            filter_name: if unchanged { "passthrough" } else { "git" },
            evidence: output.evidence,
        };
    }

    if let Ok(output) = crate::filters::test_tools::dispatch_streams_argv(
        &argv_bytes,
        &captured.stdout,
        &captured.stderr,
        captured.exit_code,
        lossless,
    ) && let Some(filtered) = changed_filtered_streams(output, captured, "test-tools")
    {
        return filtered;
    }

    if let Ok(output) = crate::filters::listing::dispatch_streams_argv(
        &argv_bytes,
        &captured.stdout,
        &captured.stderr,
        captured.exit_code,
        lossless,
    ) && let Some(filtered) = changed_filtered_streams(output, captured, "listing")
    {
        return filtered;
    }

    if let Ok(output) = crate::filters::build::dispatch_streams_argv(
        &argv_bytes,
        &captured.stdout,
        &captured.stderr,
        captured.exit_code,
        lossless,
    ) && let Some(filtered) = changed_filtered_streams(output, captured, "build")
    {
        return filtered;
    }

    if let Ok(output) = crate::filters::package::dispatch_streams_argv(
        &argv_bytes,
        &captured.stdout,
        &captured.stderr,
        captured.exit_code,
        lossless,
    ) && let Some(filtered) = changed_filtered_streams(output, captured, "package")
    {
        return filtered;
    }

    if let Ok(output) = crate::filters::infra::dispatch_streams_argv(
        &argv_bytes,
        &captured.stdout,
        &captured.stderr,
        captured.exit_code,
        lossless,
    ) && let Some(filtered) = changed_filtered_streams(output, captured, "infra")
    {
        return filtered;
    }

    if let Ok(output) = crate::filters::data::dispatch_streams_argv(
        &argv_bytes,
        &captured.stdout,
        &captured.stderr,
        captured.exit_code,
        lossless,
    ) && let Some(filtered) = changed_filtered_streams(output, captured, "data")
    {
        return filtered;
    }

    if let Ok(output) = crate::filters::diagnostics::dispatch_streams_argv(
        &argv_bytes,
        &captured.stdout,
        &captured.stderr,
        captured.exit_code,
        lossless,
    ) && let Some(filtered) = changed_filtered_streams(output, captured, "diagnostics")
    {
        return filtered;
    }

    let result = crate::pipeline::filter(&captured.stdout);
    FilteredStreams {
        stdout: result.bytes,
        stderr: Cow::Borrowed(&captured.stderr),
        filter_name: result.filter_name,
        evidence: result.evidence,
    }
}

fn changed_filtered_streams<'a>(
    output: crate::filters::StreamFilterOutput,
    captured: &'a capture::CapturedOutput,
    filter_name: &'static str,
) -> Option<FilteredStreams<'a>> {
    if output.evidence == EvidenceClass::ByteExact
        && output.stdout == captured.stdout
        && output.stderr == captured.stderr
    {
        return None;
    }
    Some(FilteredStreams {
        stdout: Cow::Owned(output.stdout),
        stderr: Cow::Owned(output.stderr),
        filter_name,
        evidence: output.evidence,
    })
}

fn passthrough_streams(captured: &capture::CapturedOutput) -> FilteredStreams<'_> {
    FilteredStreams {
        stdout: Cow::Borrowed(&captured.stdout),
        stderr: Cow::Borrowed(&captured.stderr),
        filter_name: "passthrough",
        evidence: EvidenceClass::ByteExact,
    }
}

fn command_is(argv: &[OsString], expected: &[u8]) -> bool {
    argv.first()
        .and_then(|program| crate::catalog::command_basename(program))
        .is_some_and(|name| name.as_encoded_bytes() == expected)
}

fn write_incomplete_diagnostic(incomplete: bool, stderr: &mut dyn Write) -> io::Result<usize> {
    if !incomplete {
        return Ok(0);
    }
    stderr.write_all(INCOMPLETE_OUTPUT_DIAGNOSTIC)?;
    Ok(INCOMPLETE_OUTPUT_DIAGNOSTIC.len())
}

fn return_report(
    report: RunReport,
    stderr: &mut dyn Write,
    explain: bool,
) -> io::Result<RunReport> {
    if explain {
        write_explain(&report, stderr)?;
    }
    Ok(report)
}

fn write_explain(report: &RunReport, stderr: &mut dyn Write) -> io::Result<()> {
    let omitted = report.input_bytes.saturating_sub(
        report
            .displayed_bytes
            .saturating_sub(report.diagnostic_bytes),
    );
    let saved = omitted
        .saturating_mul(100)
        .checked_div(report.input_bytes)
        .unwrap_or(0);
    writeln!(
        stderr,
        "\n(tapas explain: filter={} raw={} displayed={} omitted={} diagnostics={} saved={}% exit={} history=not-recorded)",
        report.filter_name,
        report.input_bytes,
        report.displayed_bytes,
        omitted,
        report.diagnostic_bytes,
        saved,
        report.exit_code,
    )
}

fn no_output_hint(argv: &[OsString], exit_code: i32) -> Vec<u8> {
    let command = argv
        .first()
        .and_then(|program| crate::catalog::command_basename(program.as_os_str()))
        .map_or(b"command".as_slice(), |name| name.as_encoded_bytes());
    if exit_code == 0
        && command == b"git"
        && argv
            .get(1)
            .is_some_and(|subcommand| matches!(subcommand.as_encoded_bytes(), b"status" | b"diff"))
    {
        let mut hint = b"(tapas: no changes; git ".to_vec();
        hint.extend_from_slice(argv[1].as_encoded_bytes());
        hint.extend_from_slice(b" exited 0 with no output)\n");
        return hint;
    }
    let mut hint = b"(tapas: ".to_vec();
    hint.extend_from_slice(command);
    hint.extend_from_slice(b" exited ");
    hint.extend_from_slice(exit_code.to_string().as_bytes());
    hint.extend_from_slice(b" with no output)\n");
    hint
}
