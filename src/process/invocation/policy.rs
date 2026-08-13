pub fn requests_exact_output(argv: &[OsString]) -> bool {
    with_bytes(argv, crate::invocation_policy::requests_exact_output)
}

pub fn is_raw_curl(argv: &[OsString]) -> bool {
    argv.first()
        .is_some_and(|command| basename(command) == b"curl")
        && !argv
            .iter()
            .take_while(|argument| bytes(argument) != b"--")
            .any(|argument| {
                let argument = bytes(argument);
                argument == b"--verbose"
                    || (argument.starts_with(b"-")
                        && !argument.starts_with(b"--")
                        && argument[1..].contains(&b'v'))
            })
}

pub(super) fn exact_output_reason(_command: &[u8], argv: &[OsString]) -> Option<PassthroughReason> {
    with_bytes(argv, |argv| {
        if crate::invocation_policy::requests_query(argv) {
            Some(PassthroughReason::Query)
        } else if crate::invocation_policy::requests_machine_output(argv) {
            Some(PassthroughReason::MachineOutput)
        } else {
            None
        }
    })
}

pub(super) fn is_follow_logs(command: &[u8], argv: &[OsString]) -> bool {
    if !has_follow_arg(argv) {
        return false;
    }
    command == b"docker"
        && (equals_at(argv, 1, b"logs")
            || equals_at(argv, 1, b"compose") && equals_at(argv, 2, b"logs"))
        || command == b"docker-compose" && equals_at(argv, 1, b"logs")
        || command == b"kubectl" && equals_at(argv, 1, b"logs")
        || is_any(command, &[b"tail", b"journalctl"])
}

fn has_follow_arg(argv: &[OsString]) -> bool {
    argv.iter().any(|argument| {
        let argument = bytes(argument);
        if is_any(argument, &[b"--follow", b"-f"]) {
            return true;
        }
        let Some(value) = argument.strip_prefix(b"--follow=") else {
            return false;
        };
        !is_any_ascii_case(value, &[b"0", b"false", b"no"])
    })
}

fn with_bytes<T>(argv: &[OsString], policy: impl FnOnce(&[&[u8]]) -> T) -> T {
    let argv = argv
        .iter()
        .map(|argument| argument.as_encoded_bytes())
        .collect::<Vec<_>>();
    policy(&argv)
}
use super::runners::is_any_ascii_case;
use super::{PassthroughReason, basename, bytes, equals_at, is_any};
use std::ffi::OsString;
