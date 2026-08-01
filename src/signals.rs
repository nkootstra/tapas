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
    (b"test", CARGO_TEST),
    (b"Test", JEST),
    (b" passing", JS_TEST),
    (b" failing", JS_TEST),
    (b"# tests ", JS_TEST),
    (b"error TS", TSC),
    (b"Found ", TSC),
    (b"=== ", GO_TEST),
    (b"--- ", GO_TEST),
    (b"Benchmark", GO_TEST),
    (b"ok  \t", GO_TEST),
    (b"FAIL\t", GO_TEST),
    (b"test session starts", PYTEST),
    (b"collected ", PYTEST),
    (b"passed in ", PYTEST),
    (b"failed in ", PYTEST),
    (b"added ", NPM_INSTALL),
    (b"up to date", NPM_INSTALL),
    (b"audited ", NPM_INSTALL),
    (b"npm ", NPM_INSTALL),
    (b"Packages: ", NPM_INSTALL),
    (b"packages installed", NPM_INSTALL),
    (b"success Saved ", NPM_INSTALL),
    (b"Done in ", NPM_INSTALL),
    (b"Package operations:", NPM_INSTALL),
    (b"Lock file operations:", NPM_INSTALL),
    (b"Nothing to install", NPM_INSTALL),
    (b"No security vulnerability", NPM_INSTALL),
    (b"Your requirements could not be resolved", NPM_INSTALL),
];

const FIRST_BYTE_MASK: [u32; 256] = build_first_byte_mask();
const ALL_SIGNALS: u32 = CARGO_TEST | JEST | JS_TEST | TSC | GO_TEST | PYTEST | NPM_INSTALL;

const fn build_first_byte_mask() -> [u32; 256] {
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
            let candidates = FIRST_BYTE_MASK[input[offset] as usize] & !bits;
            if candidates == 0 {
                continue;
            }
            for &(needle, bit) in NEEDLES {
                if candidates & bit != 0
                    && input[offset] == needle[0]
                    && input[offset..].starts_with(needle)
                {
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
