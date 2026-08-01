pub mod generic;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterError {
    InvalidInput,
}
