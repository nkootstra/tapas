use super::append_line;
use crate::filters::contains_ignore_ascii_case;

pub(super) fn ctest_route(argv: &[&[u8]]) -> bool {
    argv.first() == Some(&b"ctest".as_slice())
}

pub(super) fn matches_ctest(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr].into_iter().any(|input| {
        input.split(|byte| *byte == b'\n').any(|line| {
            line.starts_with(b"100% tests passed")
                || line.starts_with(b"Total Test time")
                || line.contains(&b'%') && line.contains(&b'#')
                || contains_ignore_ascii_case(line, b"tests failed")
        })
    })
}

pub(super) fn compact_ctest(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let line = raw.trim_ascii_end().trim_ascii_start();
            if line.starts_with(b"100% tests passed")
                || line.starts_with(b"Total Test time")
                || contains_ignore_ascii_case(line, b"tests failed")
                || contains_ignore_ascii_case(line, b"error")
                || line.contains(&b'%')
                    && line.contains(&b'#')
                    && !line.windows(b"Passed".len()).any(|part| part == b"Passed")
            {
                append_line(&mut output, line);
            }
        }
    }
    output
}

pub(super) fn playwright_route(argv: &[&[u8]]) -> bool {
    argv.get(1) == Some(&b"test".as_slice())
        && reporter(argv).is_some_and(|reporter| matches!(reporter, b"list" | b"line" | b"dot"))
}

pub(super) fn matches_playwright(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr].into_iter().any(|input| {
        let mut lines = input
            .split(|byte| *byte == b'\n')
            .map(|line| line.trim_ascii());
        let has_run = lines.clone().any(playwright_running_line);
        has_run && lines.any(playwright_summary)
    })
}

pub(super) fn compact_playwright(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let line = raw.trim_ascii_end().trim_ascii_start();
            if line.starts_with(b"Running ")
                || playwright_summary(line)
                || contains_ignore_ascii_case(line, b"error:")
                || line.starts_with(b"at ")
            {
                append_line(&mut output, line);
            }
        }
    }
    output
}

fn reporter<'a>(argv: &'a [&'a [u8]]) -> Option<&'a [u8]> {
    let args = crate::invocation_policy::options(argv);
    let mut reporter = None;
    for (index, argument) in args.iter().enumerate() {
        if *argument == b"--reporter" {
            let value = args.get(index + 1).copied()?;
            if reporter.replace(value).is_some() {
                return None;
            }
        }
        if let Some(value) = argument.strip_prefix(b"--reporter=")
            && reporter.replace(value).is_some()
        {
            return None;
        }
    }
    reporter
}

fn playwright_running_line(line: &[u8]) -> bool {
    let mut words = line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|word| !word.is_empty());
    words.next() == Some(b"Running".as_slice())
        && words
            .next()
            .is_some_and(|word| word.iter().all(u8::is_ascii_digit))
        && words
            .next()
            .is_some_and(|word| matches!(word, b"test" | b"tests"))
        && words.next() == Some(b"using".as_slice())
        && words
            .next()
            .is_some_and(|word| word.iter().all(u8::is_ascii_digit))
        && words
            .next()
            .is_some_and(|word| matches!(word, b"worker" | b"workers"))
        && words.next().is_none()
}

fn playwright_summary(line: &[u8]) -> bool {
    let mut words = line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|word| !word.is_empty());
    let count = words
        .next()
        .is_some_and(|word| word.iter().all(u8::is_ascii_digit));
    let status = words
        .next()
        .is_some_and(|word| matches!(word, b"passed" | b"failed" | b"skipped" | b"flaky"));
    let suffix = words.next();
    count
        && status
        && suffix.is_none_or(|word| word.starts_with(b"(") && word.ends_with(b")"))
        && words.next().is_none()
}
