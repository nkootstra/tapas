#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Signals {
    bits: u32,
}

const CARGO_TEST: u32 = 1 << 0;
const JEST: u32 = 1 << 1;
const JS_TEST: u32 = 1 << 2;
const TSC: u32 = 1 << 3;
const GO_TEST: u32 = 1 << 4;
const PYTEST: u32 = 1 << 5;
const NPM_INSTALL: u32 = 1 << 6;

const NEEDLES: &[(&[u8], u32)] = &[
    (b" passing", JS_TEST),
    (b" failing", JS_TEST),
    (b"# tests ", JS_TEST),
    (b"--- ", GO_TEST),
    (b"=== ", GO_TEST),
    (b"Benchmark", GO_TEST),
    (b"Done in ", NPM_INSTALL),
    (b"Found ", TSC),
    (b"FAIL\t", GO_TEST),
    (b"Lock file operations:", NPM_INSTALL),
    (b"Nothing to install", NPM_INSTALL),
    (b"No security vulnerability", NPM_INSTALL),
    (b"Packages: ", NPM_INSTALL),
    (b"Package operations:", NPM_INSTALL),
    (b"Test", JEST),
    (b"Your requirements could not be resolved", NPM_INSTALL),
    (b"added ", NPM_INSTALL),
    (b"audited ", NPM_INSTALL),
    (b"collected ", PYTEST),
    (b"error TS", TSC),
    (b"failed in ", PYTEST),
    (b"npm ", NPM_INSTALL),
    (b"ok  \t", GO_TEST),
    (b"passed in ", PYTEST),
    (b"packages installed", NPM_INSTALL),
    (b"success Saved ", NPM_INSTALL),
    (b"test", CARGO_TEST),
    (b"test session starts", PYTEST),
    (b"up to date", NPM_INSTALL),
];

const NO_NEEDLE: u8 = u8::MAX;
const FIRST_NEEDLE: [u8; 256] = build_first_needle();
const PAST_LAST_NEEDLE: [u8; 256] = build_past_last_needle();
const SIGNAL_BITS_BY_FIRST_BYTE: [u32; 256] = build_signal_bits_by_first_byte();
const ALL_SIGNALS: u32 = CARGO_TEST | JEST | JS_TEST | TSC | GO_TEST | PYTEST | NPM_INSTALL;

const fn build_first_needle() -> [u8; 256] {
    let mut table = [NO_NEEDLE; 256];
    let mut index = 0;
    while index < NEEDLES.len() {
        let first_byte = NEEDLES[index].0[0] as usize;
        if table[first_byte] == NO_NEEDLE {
            table[first_byte] = index as u8;
        }
        index += 1;
    }
    table
}

const fn build_past_last_needle() -> [u8; 256] {
    let mut table = [0_u8; 256];
    let mut index = 0;
    while index < NEEDLES.len() {
        table[NEEDLES[index].0[0] as usize] = (index + 1) as u8;
        index += 1;
    }
    table
}

const fn build_signal_bits_by_first_byte() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut index = 0;
    while index < NEEDLES.len() {
        let (needle, bit) = NEEDLES[index];
        table[needle[0] as usize] |= bit;
        index += 1;
    }
    table
}

impl Signals {
    pub fn compute(input: &[u8]) -> Self {
        let mut bits = 0_u32;
        for offset in 0..input.len() {
            let first = FIRST_NEEDLE[input[offset] as usize];
            if first == NO_NEEDLE {
                continue;
            }
            if bits & SIGNAL_BITS_BY_FIRST_BYTE[input[offset] as usize]
                == SIGNAL_BITS_BY_FIRST_BYTE[input[offset] as usize]
            {
                continue;
            }
            let past_last = PAST_LAST_NEEDLE[input[offset] as usize];
            for &(needle, bit) in &NEEDLES[first as usize..past_last as usize] {
                if bits & bit == 0 && input[offset..].starts_with(needle) {
                    bits |= bit;
                }
            }
            if bits == ALL_SIGNALS {
                break;
            }
        }
        Self { bits }
    }

    pub fn cargo_test(self) -> bool {
        self.bits & CARGO_TEST != 0
    }

    pub fn jest(self) -> bool {
        self.bits & JEST != 0
    }

    pub fn js_test(self) -> bool {
        self.bits & JS_TEST != 0
    }

    pub fn tsc(self) -> bool {
        self.bits & TSC != 0
    }

    pub fn go_test(self) -> bool {
        self.bits & GO_TEST != 0
    }

    pub fn pytest(self) -> bool {
        self.bits & PYTEST != 0
    }

    pub fn npm_install(self) -> bool {
        self.bits & NPM_INSTALL != 0
    }
}
