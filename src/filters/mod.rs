pub mod build;
pub mod data;
pub mod diagnostics;
pub mod generic;
pub mod git;
pub mod infra;
pub mod listing;
pub mod package;
pub mod test_tools;
mod util;

pub(crate) use crate::catalog::command_basename_bytes as command_basename;
pub(crate) use util::{
    append_line, byte_after_lines, contains_ignore_ascii_case, find_subslice, normalize_log_line,
    rfind_subslice, strip_ansi, strip_ansi_csi, timestamp_end, trim_ascii_end_space,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceClass {
    ByteExact,
    FactComplete,
    PotentiallyLossy,
}

#[derive(Debug, Eq, PartialEq)]
pub struct FilterOutput {
    pub bytes: Vec<u8>,
    pub evidence: EvidenceClass,
}

impl FilterOutput {
    pub fn new(bytes: Vec<u8>, evidence: EvidenceClass) -> Self {
        Self { bytes, evidence }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct StreamFilterOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub evidence: EvidenceClass,
}

#[derive(Clone, Copy)]
pub(crate) struct StreamFilterInput<'a> {
    pub(crate) argv: &'a [&'a [u8]],
    pub(crate) stdout: &'a [u8],
    pub(crate) stderr: &'a [u8],
    pub(crate) exit_code: i32,
    pub(crate) lossless: bool,
}

impl<'a> StreamFilterInput<'a> {
    pub(crate) fn new(
        argv: &'a [&'a [u8]],
        stdout: &'a [u8],
        stderr: &'a [u8],
        exit_code: i32,
        lossless: bool,
    ) -> Self {
        Self {
            argv,
            stdout,
            stderr,
            exit_code,
            lossless,
        }
    }
}

/// Internal routing result that lets dispatchers represent passthrough without
/// an owned copy of the captured streams.
pub(crate) enum StreamFilterDecision {
    /// The family recognized and owns this route, but its output must remain
    /// byte-exact instead of falling through to another filter.
    #[allow(dead_code)] // Route implementations adopt this foundation incrementally.
    Passthrough,
    /// The family did not apply a route-specific decision. Dispatch may use
    /// the family's configured fallback behavior.
    Unchanged,
    Applied(StreamFilterOutput),
}

impl StreamFilterOutput {
    pub fn new(stdout: Vec<u8>, stderr: Vec<u8>, evidence: EvidenceClass) -> Self {
        Self {
            stdout,
            stderr,
            evidence,
        }
    }

    pub fn passthrough(stdout: &[u8], stderr: &[u8]) -> Self {
        Self::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact)
    }

    pub(crate) fn compact_single_stream(
        stdout: &[u8],
        stderr: &[u8],
        evidence: EvidenceClass,
        compact: impl FnOnce(&[u8], &[u8]) -> Vec<u8>,
    ) -> Self {
        match (stdout.is_empty(), stderr.is_empty()) {
            (false, false) => Self::passthrough(stdout, stderr),
            (false, true) => Self::new(compact(stdout, b""), Vec::new(), evidence),
            (true, false) => Self::new(Vec::new(), compact(b"", stderr), evidence),
            (true, true) => Self::new(compact(b"", b""), Vec::new(), evidence),
        }
    }
}

impl StreamFilterDecision {
    pub(crate) fn compact_single_stream(
        stdout: &[u8],
        stderr: &[u8],
        evidence: EvidenceClass,
        compact: impl FnOnce(&[u8], &[u8]) -> Vec<u8>,
    ) -> Self {
        match (stdout.is_empty(), stderr.is_empty()) {
            (false, false) => Self::Unchanged,
            (false, true) => Self::Applied(StreamFilterOutput::new(
                compact(stdout, b""),
                Vec::new(),
                evidence,
            )),
            (true, false) => Self::Applied(StreamFilterOutput::new(
                Vec::new(),
                compact(b"", stderr),
                evidence,
            )),
            (true, true) => Self::Applied(StreamFilterOutput::new(
                compact(b"", b""),
                Vec::new(),
                evidence,
            )),
        }
    }

    pub(crate) fn into_output(self, stdout: &[u8], stderr: &[u8]) -> StreamFilterOutput {
        match self {
            Self::Passthrough | Self::Unchanged => StreamFilterOutput::passthrough(stdout, stderr),
            Self::Applied(output) => output,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterError {
    InvalidInput,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_decision_converts_to_byte_exact_output() {
        let output = StreamFilterDecision::Passthrough.into_output(b"stdout\0\xff", b"stderr\n");

        assert_eq!(output.stdout, b"stdout\0\xff");
        assert_eq!(output.stderr, b"stderr\n");
        assert_eq!(output.evidence, EvidenceClass::ByteExact);
    }
}
