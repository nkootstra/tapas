pub(super) fn compact_dotnet(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            if raw.is_empty() {
                continue;
            }
            let clean = strip_ansi(raw);
            let line = trim_ascii_end(&clean);
            if should_keep_dotnet(line) {
                append_line(&mut output, line);
            }
        }
    }
    output
}

fn should_keep_dotnet(line: &[u8]) -> bool {
    let trimmed = trim_ascii(line);
    if trimmed.is_empty() {
        return false;
    }
    contains_ignore_ascii_case(trimmed, b": error ")
        || contains_ignore_ascii_case(trimmed, b": warning ")
        || find_subslice(trimmed, b" error CS").is_some()
        || find_subslice(trimmed, b" warning CS").is_some()
        || contains_ignore_ascii_case(trimmed, b"build failed")
        || contains_ignore_ascii_case(trimmed, b"build succeeded")
        || trimmed.ends_with(b" Error(s)")
        || trimmed.ends_with(b" Warning(s)")
        || trimmed.starts_with(b"Restored ")
        || contains_ignore_ascii_case(trimmed, b"restore failed")
        || contains_ignore_ascii_case(trimmed, b"restore succeeded")
        || contains_ignore_ascii_case(trimmed, b"test run failed")
        || trimmed.starts_with(b"[xUnit.net ") && find_subslice(trimmed, b"[FAIL]").is_some()
        || trimmed.starts_with(b"Failed ")
        || contains_ignore_ascii_case(trimmed, b"error message:")
        || find_subslice(trimmed, b"Assert.").is_some()
        || trimmed.starts_with(b"Expected:")
        || trimmed.starts_with(b"Actual:")
        || contains_ignore_ascii_case(trimmed, b"stack trace:")
        || trimmed.starts_with(b"at ") && find_subslice(trimmed, b".cs:line ").is_some()
        || contains_ignore_ascii_case(trimmed, b"failed!")
        || contains_ignore_ascii_case(trimmed, b"passed!")
        || contains_ignore_ascii_case(trimmed, b"failed:")
        || contains_ignore_ascii_case(trimmed, b"passed:")
        || contains_ignore_ascii_case(trimmed, b"total tests:")
        || contains_ignore_ascii_case(trimmed, b"failed tests:")
        || contains_ignore_ascii_case(trimmed, b"format complete")
        || contains_ignore_ascii_case(trimmed, b"formatted code file")
        || contains_ignore_ascii_case(trimmed, b"would be formatted")
}

pub(super) fn compact_evidence(exit_code: i32) -> EvidenceClass {
    if exit_code == 0 {
        EvidenceClass::PotentiallyLossy
    } else {
        EvidenceClass::FactComplete
    }
}

pub(super) fn has_recognized_failure(input: &[u8]) -> bool {
    [
        b"error:".as_slice(),
        b"Error:",
        b": error ",
        b" error CS",
        b"[ERROR]",
        b"ERROR ",
        b"FAILURE:",
        b"BUILD FAILED",
        b"BUILD FAILURE",
        b"Build FAILED",
        b"Test run failed",
        b"[FAIL]",
        b"Failed!",
        b" FAILED",
        b"Exception",
        b"AssertionError",
        b"** BUILD FAILED **",
        b"** TEST FAILED **",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

pub(super) fn matches_build_output(input: &[u8]) -> bool {
    find_subslice(input, b"Tasks:").is_some()
        && find_subslice(input, b"Duration:").is_some()
        && (find_subslice(input, b"\n> ").is_some() || input.starts_with(b"> "))
        || find_subslice(input, b"vite v").is_some()
            && (find_subslice(input, b"building for production").is_some()
                || find_subslice(input, b"building SSR bundle").is_some())
        || find_subslice(input, b"\xe2\x96\xb2 Next.js").is_some()
        || find_subslice(input, b"Creating an optimized production build").is_some()
        || find_subslice(input, b"Compiled successfully").is_some()
        || find_subslice(input, b"Nuxt ").is_some() && find_subslice(input, b"with Nitro").is_some()
        || find_subslice(input, b"webpack ").is_some()
            && find_subslice(input, b" compiled ").is_some()
        || find_subslice(input, b"modules transformed").is_some()
        || find_subslice(input, b"\xe2\x9c\x93 built in ").is_some()
        || find_subslice(input, b"\xce\xa3 Total size:").is_some()
}
use super::exact::{strip_ansi, trim_ascii};
use super::{
    EvidenceClass, append_line, contains_ignore_ascii_case, find_subslice, trim_ascii_end,
};
