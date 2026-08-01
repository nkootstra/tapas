use std::io::{self, Read, Write};

use crate::filters::generic;
use crate::filters::{FilterError, FilterOutput};
use crate::signals::Signals;

pub const MAX_PIPE_INPUT_BYTES: usize = 16 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 32 * 1024;

pub type GateFn = fn(Signals) -> bool;
pub type MatchFn = fn(&[u8]) -> bool;
pub type ApplyFn = fn(&[u8]) -> Result<FilterOutput, FilterError>;

#[derive(Clone, Copy)]
pub struct FilterSpec {
    pub name: &'static str,
    gate: GateFn,
    matches: MatchFn,
    apply: ApplyFn,
}

impl FilterSpec {
    pub const fn new(name: &'static str, gate: GateFn, matches: MatchFn, apply: ApplyFn) -> Self {
        Self {
            name,
            gate,
            matches,
            apply,
        }
    }
}

const DEFAULT_FILTERS: &[FilterSpec] = &[FilterSpec::new(
    "generic",
    generic::always,
    generic::matches,
    generic::apply,
)];

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
    dispatch_with_filters(input, DEFAULT_FILTERS)
}

pub fn dispatch_with_filters(input: &[u8], filters: &[FilterSpec]) -> Vec<u8> {
    let signals = Signals::compute(input);
    for filter in filters {
        if !(filter.gate)(signals) || !(filter.matches)(input) {
            continue;
        }
        return (filter.apply)(input)
            .map(|candidate| candidate.bytes)
            .unwrap_or_else(|_| input.to_vec());
    }
    input.to_vec()
}
