use super::{append_line, contains_ignore_ascii_case, find_subslice, trim_ascii_end};

pub(super) fn vite_route(argv: &[&[u8]]) -> bool {
    argv.get(1) == Some(&b"build".as_slice()) && !has_before_terminator(argv, &[b"--watch"])
}

pub(super) fn matches_vite(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr].into_iter().any(|input| {
        find_subslice(input, b"vite v").is_some()
            && find_subslice(input, b"building for production").is_some()
            && (find_subslice(input, b"modules transformed").is_some()
                || find_subslice(input, b"built in").is_some()
                || contains_ignore_ascii_case(input, b"error"))
    })
}

pub(super) fn esbuild_route(argv: &[&[u8]]) -> bool {
    has_output_option(argv)
        && !has_before_terminator(argv, &[b"--watch", b"--serve"])
        && !has_before_terminator(argv, &[b"--outfile=-", b"--outdir=-"])
}

pub(super) fn matches_esbuild(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr].into_iter().any(|input| {
        input.split(|byte| *byte == b'\n').any(|line| {
            line.starts_with(b"\xe2\x9a\xa1 Done in ")
                || contains_ignore_ascii_case(line, b"error:")
                || esbuild_size_line(line)
        })
    })
}

pub(super) fn compact_esbuild(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let line = trim_ascii_end(raw).trim_ascii_start();
            if line.is_empty() {
                continue;
            }
            if let Some(done) = line.strip_prefix(b"\xe2\x9a\xa1 ") {
                append_line(&mut output, done);
            } else if esbuild_size_line(line) || contains_ignore_ascii_case(line, b"error:") {
                append_collapsed(&mut output, line);
            }
        }
    }
    output
}

pub(super) fn cmake_route(argv: &[&[u8]]) -> bool {
    if cmake_exact(argv) {
        return false;
    }
    argv.get(1) == Some(&b"--build".as_slice())
        || argv.iter().skip(1).any(|argument| *argument == b"-S")
            && argv.iter().skip(1).any(|argument| *argument == b"-B")
        || argv
            .get(1)
            .is_some_and(|argument| !argument.starts_with(b"-"))
}

pub(super) fn cmake_exact(argv: &[&[u8]]) -> bool {
    crate::invocation_policy::options(argv)
        .iter()
        .any(|argument| {
            matches!(*argument, b"-E" | b"-P" | b"--system-information")
                || argument.starts_with(b"--trace")
                || argument.starts_with(b"--find-package")
                || argument.starts_with(b"--graphviz")
                || argument.starts_with(b"--workflow")
                || argument.starts_with(b"--list-presets")
        })
}

pub(super) fn matches_cmake(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr].into_iter().any(|input| {
        input.split(|byte| *byte == b'\n').any(|line| {
            line.starts_with(b"-- Configuring done")
                || line.starts_with(b"-- Generating done")
                || line.starts_with(b"-- Build files have been written to:")
                || line.starts_with(b"[") && line.contains(&b'%')
                || contains_ignore_ascii_case(line, b"cmake error")
        })
    })
}

pub(super) fn compact_cmake(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let line = trim_ascii_end(raw);
            if line.starts_with(b"-- Configuring done")
                || line.starts_with(b"-- Generating done")
                || line.starts_with(b"-- Build files have been written to:")
                || contains_ignore_ascii_case(line, b"error")
                || contains_ignore_ascii_case(line, b"warning")
                || line.starts_with(b"[") && line.contains(&b'%')
            {
                append_line(&mut output, line);
            }
        }
    }
    output
}

fn has_output_option(argv: &[&[u8]]) -> bool {
    let args = crate::invocation_policy::options(argv);
    args.iter().enumerate().any(|(index, argument)| {
        argument.starts_with(b"--outfile=")
            || argument.starts_with(b"--outdir=")
            || matches!(*argument, b"--outfile" | b"--outdir") && args.get(index + 1).is_some()
    })
}

fn has_before_terminator(argv: &[&[u8]], options: &[&[u8]]) -> bool {
    crate::invocation_policy::options(argv)
        .iter()
        .any(|argument| {
            options.iter().any(|option| {
                *argument == *option
                    || argument
                        .strip_prefix(*option)
                        .is_some_and(|rest| rest.starts_with(b"="))
            })
        })
}

fn esbuild_size_line(line: &[u8]) -> bool {
    let line = line.trim_ascii();
    let mut tokens = line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|token| !token.is_empty());
    tokens.next().is_some()
        && tokens
            .next()
            .is_some_and(|size| size.ends_with(b"b") && size.iter().any(u8::is_ascii_digit))
}

fn append_collapsed(output: &mut Vec<u8>, line: &[u8]) {
    let mut first = true;
    for token in line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|token| !token.is_empty())
    {
        if !first {
            output.push(b' ');
        }
        first = false;
        output.extend_from_slice(token);
    }
    output.push(b'\n');
}
