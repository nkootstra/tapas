pub(super) const UV_VALUE: &[&[u8]] = &[
    b"--project",
    b"--directory",
    b"--python",
    b"--package",
    b"--with",
    b"--with-editable",
    b"--with-requirements",
    b"--env-file",
    b"--group",
    b"--extra",
];
pub(super) const UVX_VALUE: &[&[u8]] = &[
    b"--project",
    b"--directory",
    b"--python",
    b"--package",
    b"--with",
    b"--with-editable",
    b"--with-requirements",
    b"--env-file",
    b"--group",
    b"--extra",
    b"--from",
];
pub(super) const UV_BOOLEAN: &[&[u8]] = &[
    b"--isolated",
    b"--active",
    b"--no-sync",
    b"--locked",
    b"--frozen",
    b"--no-project",
    b"--all-extras",
    b"--no-dev",
    b"--no-progress",
    b"--offline",
];

pub(super) fn unwrap_direct<'a>(
    argv: &'a [OsString],
    start: usize,
    values: &[&[u8]],
    booleans: &[&[u8]],
    opaque: &[&[u8]],
) -> Option<&'a [OsString]> {
    let index = scan_options(argv, start, values, booleans, opaque, None)?;
    tool_slice(argv, index)
}

pub(super) fn unwrap_subcommand<'a>(
    argv: &'a [OsString],
    start: usize,
    subcommand: &[u8],
    values: &[&[u8]],
    booleans: &[&[u8]],
) -> Option<&'a [OsString]> {
    let index = scan_options(argv, start, values, booleans, &[], Some(subcommand))?;
    if !equals_at(argv, index, subcommand) {
        return None;
    }
    tool_slice(argv, index + 1)
}

fn scan_options(
    argv: &[OsString],
    mut index: usize,
    values: &[&[u8]],
    booleans: &[&[u8]],
    opaque: &[&[u8]],
    stop: Option<&[u8]>,
) -> Option<usize> {
    while index < argv.len() {
        let argument = bytes(&argv[index]);
        if stop == Some(argument) {
            return Some(index);
        }
        if argument == b"--" {
            return stop.is_none().then_some(index + 1);
        }
        if argument.is_empty() || argument[0] != b'-' {
            return stop.is_none().then_some(index);
        }
        if option_match(argument, opaque) != OptionMatch::None {
            return None;
        }
        if matches_boolean(argument, booleans) {
            index += 1;
            continue;
        }
        match option_match(argument, values) {
            OptionMatch::Inline => index += 1,
            OptionMatch::Separate if index + 1 < argv.len() => index += 2,
            _ => return None,
        }
    }
    None
}

fn tool_slice(argv: &[OsString], mut index: usize) -> Option<&[OsString]> {
    if equals_at(argv, index, b"--") {
        index += 1;
    }
    argv.get(index)
        .is_some_and(|argument| !bytes(argument).is_empty() && !bytes(argument).starts_with(b"-"))
        .then_some(&argv[index..])
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OptionMatch {
    None,
    Separate,
    Inline,
}

fn option_match(argument: &[u8], options: &[&[u8]]) -> OptionMatch {
    for option in options {
        if argument == *option {
            return OptionMatch::Separate;
        }
        if option.len() > 2
            && option.starts_with(b"--")
            && argument
                .strip_prefix(*option)
                .is_some_and(|rest| rest.starts_with(b"="))
            || option.len() == 2
                && option.starts_with(b"-")
                && argument.len() > 2
                && argument.starts_with(option)
        {
            return OptionMatch::Inline;
        }
    }
    OptionMatch::None
}

fn matches_boolean(argument: &[u8], options: &[&[u8]]) -> bool {
    match option_match(argument, options) {
        OptionMatch::None => false,
        OptionMatch::Separate => true,
        OptionMatch::Inline => {
            argument.len() > 2
                && argument[0] == b'-'
                && argument[1] != b'-'
                && argument[1..].iter().all(|flag| {
                    options
                        .iter()
                        .any(|option| option.len() == 2 && option[1] == *flag)
                })
        }
    }
}

pub(super) fn has_any_arg(argv: &[OsString], needles: &[&[u8]]) -> bool {
    argv.iter()
        .take_while(|argument| bytes(argument) != b"--")
        .any(|argument| is_any(bytes(argument), needles))
}

pub(super) fn has_arg(argv: &[OsString], needle: &[u8]) -> bool {
    argv.iter().any(|argument| bytes(argument) == needle)
}

pub(super) fn equals_at(argv: &[OsString], index: usize, expected: &[u8]) -> bool {
    argv.get(index)
        .is_some_and(|argument| bytes(argument) == expected)
}

pub(super) fn is_any(value: &[u8], options: &[&[u8]]) -> bool {
    options.contains(&value)
}

pub(super) fn is_any_ascii_case(value: &[u8], options: &[&[u8]]) -> bool {
    options
        .iter()
        .any(|option| value.eq_ignore_ascii_case(option))
}

pub(super) fn basename(argument: &OsString) -> &[u8] {
    crate::catalog::command_basename(argument.as_os_str())
        .unwrap_or(argument.as_os_str())
        .as_encoded_bytes()
}

pub(super) fn bytes(argument: &OsString) -> &[u8] {
    argument.as_encoded_bytes()
}
use std::ffi::OsString;
