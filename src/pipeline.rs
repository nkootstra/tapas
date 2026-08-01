use std::borrow::Cow;
use std::io::{self, Read, Write};

use crate::filters::{FilterError, FilterOutput};
use crate::filters::{generic, git, test_tools};
use crate::signals::Signals;

pub const MAX_PIPE_INPUT_BYTES: usize = 16 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 32 * 1024;

pub type GateFn = fn(Signals) -> bool;
pub type MatchFn = fn(&[u8]) -> bool;
pub type ApplyFn = fn(&[u8]) -> Result<FilterOutput, FilterError>;

#[derive(Clone, Copy)]
pub struct FilterSpec {
    pub name: &'static str,
    gate: Option<GateFn>,
    matches: MatchFn,
    apply: ApplyFn,
}

impl FilterSpec {
    pub const fn new(name: &'static str, gate: GateFn, matches: MatchFn, apply: ApplyFn) -> Self {
        Self {
            name,
            gate: Some(gate),
            matches,
            apply,
        }
    }

    pub const fn ungated(name: &'static str, matches: MatchFn, apply: ApplyFn) -> Self {
        Self {
            name,
            gate: None,
            matches,
            apply,
        }
    }
}

const DEFAULT_FILTERS: &[FilterSpec] = &[
    FilterSpec::ungated("git", git::matches, git::apply_matched),
    FilterSpec::new(
        "test-tools",
        test_tools_gate,
        test_tools::matches,
        test_tools::apply_matched,
    ),
    FilterSpec::ungated("generic", generic::matches, generic::apply_matched),
];

fn test_tools_gate(signals: Signals) -> bool {
    signals.cargo_test()
        || signals.jest()
        || signals.js_test()
        || signals.tsc()
        || signals.go_test()
        || signals.pytest()
}

#[derive(Debug)]
pub struct DispatchResult<'a> {
    pub bytes: Cow<'a, [u8]>,
    pub filter_name: &'static str,
    pub evidence: crate::filters::EvidenceClass,
}

pub fn run(reader: &mut dyn Read, writer: &mut dyn Write) -> io::Result<()> {
    let mut retained = Vec::with_capacity(READ_BUFFER_BYTES);
    let mut chunk = [0_u8; READ_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            writer.write_all(&filter_bytes(&retained))?;
            return Ok(());
        }
        if retained.len() + read >= MAX_PIPE_INPUT_BYTES {
            writer.write_all(&retained)?;
            writer.write_all(&chunk[..read])?;
            io::copy(reader, writer)?;
            return Ok(());
        }
        retained.extend_from_slice(&chunk[..read]);
    }
}

pub fn filter_bytes(input: &[u8]) -> Vec<u8> {
    filter(input).bytes.into_owned()
}

pub fn filter(input: &[u8]) -> DispatchResult<'_> {
    dispatch(input, DEFAULT_FILTERS)
}

pub fn dispatch_with_filters(input: &[u8], filters: &[FilterSpec]) -> Vec<u8> {
    dispatch(input, filters).bytes.into_owned()
}

fn dispatch<'a>(input: &'a [u8], filters: &[FilterSpec]) -> DispatchResult<'a> {
    let mut signals = None;
    for filter in filters {
        if filter
            .gate
            .is_some_and(|gate| !gate(*signals.get_or_insert_with(|| Signals::compute(input))))
            || !(filter.matches)(input)
        {
            continue;
        }
        return match (filter.apply)(input) {
            Ok(candidate) => DispatchResult {
                bytes: Cow::Owned(candidate.bytes),
                filter_name: filter.name,
                evidence: candidate.evidence,
            },
            Err(_) => passthrough(input),
        };
    }
    passthrough(input)
}

fn passthrough(input: &[u8]) -> DispatchResult<'_> {
    DispatchResult {
        bytes: Cow::Borrowed(input),
        filter_name: "passthrough",
        evidence: crate::filters::EvidenceClass::ByteExact,
    }
}
