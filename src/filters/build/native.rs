pub(super) fn compact_build(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if let Some(summary) =
        find_zig_success_summary(stdout).or_else(|| find_zig_success_summary(stderr))
    {
        let mut output = summary.to_vec();
        output.push(b'\n');
        return output;
    }

    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut cargo_count = 0usize;
    let mut cargo_verbose_count = 0usize;
    let mut make_count = 0usize;
    let mut ninja_count = 0usize;
    let mut go_count = 0usize;
    let mut cargo_finished_dev = false;
    for input in [stdout, stderr] {
        if input.is_empty() {
            continue;
        }
        let input = input.strip_suffix(b"\n").unwrap_or(input);
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = clean.as_slice();
            if is_make_directory_noise(line) {
                continue;
            }
            match classify_build_line(line) {
                BuildLine::CargoProgress | BuildLine::CargoCheckProgress => cargo_count += 1,
                BuildLine::CargoVerboseInvocation => cargo_verbose_count += 1,
                BuildLine::MakeProgress => make_count += 1,
                BuildLine::NinjaProgress(completed) => {
                    ninja_count = ninja_count.max(completed);
                }
                BuildLine::GoProgress => go_count += 1,
                BuildLine::Kept | BuildLine::Other => {
                    if line.starts_with(b"    Finished dev") {
                        cargo_finished_dev = true;
                    } else if !is_cargo_generated_warning_summary(line) {
                        append_line(&mut output, line);
                    }
                }
            }
        }
    }
    if cargo_count > 0 {
        output.extend_from_slice(b"cargo: ");
        if cargo_finished_dev {
            output.extend_from_slice(b"Finished dev; ");
        }
        output.extend_from_slice(cargo_count.to_string().as_bytes());
        output.extend_from_slice(b" crates\n");
    } else if cargo_verbose_count > 0 {
        output.extend_from_slice(b"Ran ");
        output.extend_from_slice(cargo_verbose_count.to_string().as_bytes());
        output.extend_from_slice(b" rustc invocations (cargo -vv)\n");
    }
    if make_count > 0 {
        output.extend_from_slice(b"Compiled ");
        output.extend_from_slice(make_count.to_string().as_bytes());
        output.extend_from_slice(b" (make)\n");
    }
    if ninja_count > 0 {
        output.extend_from_slice(b"built ");
        output.extend_from_slice(ninja_count.to_string().as_bytes());
        output.extend_from_slice(b" (ninja)\n");
    }
    if go_count > 0 {
        output.extend_from_slice(b"Compiled ");
        output.extend_from_slice(go_count.to_string().as_bytes());
        output.extend_from_slice(b" (go)\n");
    }
    output
}

pub(super) fn ninja_completed(line: &[u8]) -> Option<usize> {
    if line.len() < 6 || line[0] != b'[' {
        return None;
    }
    let mut index = 1usize;
    let mut completed = 0usize;
    while line.get(index).is_some_and(u8::is_ascii_digit) {
        completed = completed * 10 + usize::from(line[index] - b'0');
        index += 1;
    }
    if index == 1 || line.get(index) != Some(&b'/') {
        return None;
    }
    index += 1;
    let total_start = index;
    while line.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == total_start || line.get(index) != Some(&b']') || line.get(index + 1) != Some(&b' ')
    {
        return None;
    }
    Some(completed)
}

fn find_zig_success_summary(input: &[u8]) -> Option<&[u8]> {
    let start = find_subslice(input, b"Build Summary: ")?;
    let rest = &input[start..];
    let end = rest
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(rest.len());
    let line = &rest[..end];
    find_subslice(line, b"failed").is_none().then_some(line)
}

fn is_make_directory_noise(line: &[u8]) -> bool {
    line.starts_with(b"make")
        && (find_subslice(line, b": Entering directory").is_some()
            || find_subslice(line, b": Leaving directory").is_some())
}

fn is_cargo_generated_warning_summary(line: &[u8]) -> bool {
    line.starts_with(b"warning: `") && find_subslice(line, b" generated ").is_some()
}
use super::exact::strip_ansi;
use super::frontend::{BuildLine, classify_build_line};
use super::{append_line, find_subslice};
