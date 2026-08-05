use std::ffi::{OsStr, OsString};

use super::spec;
use crate::setup::{Action, Target};

pub(super) enum Invocation<'a> {
    Version,
    Help,
    Filters,
    Completions(spec::Shell),
    RawInput,
    Process {
        command: &'a [OsString],
        mode: ProcessMode,
    },
    Rewrite(&'a [OsString]),
    HookEval {
        target: Target,
        self_check: bool,
    },
    Setup(SetupRequest),
    Deferred(&'a OsStr),
    UnknownOption(&'a OsStr),
    UsageError(&'static [u8]),
}

pub(super) enum ProcessMode {
    Compact,
    Raw,
    Explain,
}

pub(super) struct SetupRequest {
    pub(super) action: Action,
    pub(super) target: Target,
    pub(super) dry_run: bool,
    pub(super) force: bool,
}

pub(super) fn parse(args: &[OsString]) -> Invocation<'_> {
    let mode = spec::Mode::parse(&args[0]);
    match mode {
        Some(spec::Mode::Version) if args.len() == 1 => Invocation::Version,
        Some(spec::Mode::Version) => Invocation::UsageError(b"--version does not accept arguments"),
        Some(spec::Mode::Help) if args.len() == 1 => Invocation::Help,
        Some(spec::Mode::Help) => Invocation::UsageError(b"--help does not accept arguments"),
        Some(spec::Mode::Filters) if args.len() == 1 => Invocation::Filters,
        Some(spec::Mode::Filters) => Invocation::UsageError(b"--filters does not accept arguments"),
        Some(spec::Mode::Completions) => parse_completions(args),
        Some(spec::Mode::Raw) => parse_raw(&args[1..]),
        Some(spec::Mode::Explain) if args.len() > 1 => Invocation::Process {
            command: &args[1..],
            mode: ProcessMode::Explain,
        },
        Some(spec::Mode::Explain) => Invocation::UsageError(b"--explain requires a command"),
        Some(spec::Mode::Rewrite) if args.len() > 1 => Invocation::Rewrite(&args[1..]),
        Some(spec::Mode::Rewrite) => Invocation::UsageError(b"--rewrite requires a command"),
        Some(spec::Mode::HookEval) => parse_hook(&args[1..]),
        Some(mode @ (spec::Mode::Setup | spec::Mode::Unsetup)) => parse_setup(mode, args),
        None if is_deferred_mode(&args[0]) => Invocation::Deferred(&args[0]),
        None if args[0].as_encoded_bytes().starts_with(b"-") => Invocation::UnknownOption(&args[0]),
        None => Invocation::Process {
            command: args,
            mode: ProcessMode::Compact,
        },
    }
}

fn parse_completions(args: &[OsString]) -> Invocation<'_> {
    let [_, shell] = args else {
        return Invocation::UsageError(b"--completions requires bash, zsh, or fish");
    };
    match spec::Shell::parse(shell) {
        Some(shell) => Invocation::Completions(shell),
        None => Invocation::UsageError(b"--completions requires bash, zsh, or fish"),
    }
}

fn parse_raw(args: &[OsString]) -> Invocation<'_> {
    if args.is_empty() {
        return Invocation::RawInput;
    }
    let command = if args[0] == OsStr::new("--") {
        &args[1..]
    } else {
        args
    };
    if command.is_empty() {
        Invocation::UsageError(b"--raw requires a command after --")
    } else {
        Invocation::Process {
            command,
            mode: ProcessMode::Raw,
        }
    }
}

fn parse_hook(args: &[OsString]) -> Invocation<'_> {
    let request = match args {
        [target] => Target::parse(target).map(|target| (target, false)),
        [target, self_check] if self_check == OsStr::new("--self-check") => {
            Target::parse(target).map(|target| (target, true))
        }
        _ => None,
    };
    match request {
        Some((target, self_check)) => Invocation::HookEval { target, self_check },
        None => Invocation::UsageError(b"--hook-eval requires claude, codex, or opencode"),
    }
}

fn parse_setup(mode: spec::Mode, args: &[OsString]) -> Invocation<'_> {
    match setup_request(mode, args) {
        Some(request) => Invocation::Setup(request),
        None => Invocation::UsageError(b"invalid --setup or --unsetup arguments"),
    }
}

fn setup_request(mode: spec::Mode, args: &[OsString]) -> Option<SetupRequest> {
    let first = args.first()?.as_encoded_bytes();
    let (target, option_start) =
        if let Some(separator) = first.iter().position(|byte| *byte == b'=') {
            (&first[separator + 1..], 1)
        } else {
            (args.get(1)?.as_encoded_bytes(), 2)
        };
    let action = match mode {
        spec::Mode::Setup => Action::Setup,
        spec::Mode::Unsetup => Action::Unsetup,
        _ => unreachable!("setup_request only handles setup modes"),
    };
    let mut request = SetupRequest {
        action,
        target: Target::parse_bytes(target)?,
        dry_run: false,
        force: false,
    };
    for option in &args[option_start..] {
        match option.as_encoded_bytes() {
            b"--dry-run" if !request.dry_run => request.dry_run = true,
            b"--force" if !request.force => request.force = true,
            _ => return None,
        }
    }
    if request.force && (request.action != Action::Setup || request.target != Target::OpenCode) {
        return None;
    }
    Some(request)
}

fn is_deferred_mode(argument: &OsStr) -> bool {
    ["--stats", "--discover", "--err", "--test"]
        .iter()
        .any(|deferred| argument == OsStr::new(deferred))
}
