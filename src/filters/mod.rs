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

pub(crate) use util::{
    append_line, byte_after_lines, command_basename, contains_ignore_ascii_case, find_subslice,
    normalize_log_line, rfind_subslice, strip_ansi, strip_ansi_csi, timestamp_end,
    trim_ascii_end_space,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterError {
    InvalidInput,
}
