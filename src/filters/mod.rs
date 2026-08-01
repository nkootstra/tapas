pub mod generic;
pub mod git;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterError {
    InvalidInput,
}
