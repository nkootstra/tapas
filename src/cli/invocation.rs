use std::ffi::{OsStr, OsString};

use super::spec;
use crate::plugins::Management;
use crate::setup::{Action, SetupRequest, Target};

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
    Plugin(Management<'a>),
    Deferred(&'a OsStr),
    UnknownOption(&'a OsStr),
    UsageError(&'static [u8]),
}

pub(super) enum ProcessMode {
    Compact,
    Raw,
    Explain,
}

pub(super) fn parse(args: &[OsString]) -> Invocation<'_> {
    if args[0] == OsStr::new("--plugin") {
        return parse_plugin(args);
    }
    let mode = spec::Mode::parse(&args[0]);
    match mode {
        Some(spec::Mode::Version) if args.len() == 1 => Invocation::Version,
        Some(spec::Mode::Version) => Invocation::UsageError(b"--version does not accept arguments"),
        Some(spec::Mode::Help) if args.len() == 1 => Invocation::Help,
        Some(spec::Mode::Help) => Invocation::UsageError(b"--help does not accept arguments"),
        Some(spec::Mode::Filters) if args.len() == 1 => Invocation::Filters,
        Some(spec::Mode::Filters) => Invocation::UsageError(b"--filters does not accept arguments"),
        Some(spec::Mode::Plugin) => unreachable!("--plugin is parsed before static modes"),
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

fn parse_plugin(args: &[OsString]) -> Invocation<'_> {
    match args {
        [_, action, separator, path]
            if action == OsStr::new("check") && separator == OsStr::new("--") =>
        {
            Invocation::Plugin(Management::Check { path })
        }
        [_, action, id] if action == OsStr::new("test") => {
            Invocation::Plugin(Management::Test { id })
        }
        [_, action, id, flags @ ..] if action == OsStr::new("trust") => {
            let Some((pinned, replace, expected_sha256, path)) = parse_trust_flags(flags) else {
                return Invocation::UsageError(b"invalid --plugin arguments");
            };
            Invocation::Plugin(Management::Trust {
                id,
                path,
                pinned,
                replace,
                expected_sha256,
            })
        }
        [_, action, scope, id, separator, prefix @ ..]
            if action == OsStr::new("bind")
                && scope == OsStr::new("--user")
                && separator == OsStr::new("--")
                && !prefix.is_empty() =>
        {
            Invocation::Plugin(Management::BindUser { id, prefix })
        }
        [_, action, scope, id, separator, prefix @ ..]
            if action == OsStr::new("bind")
                && scope == OsStr::new("--project")
                && separator == OsStr::new("--")
                && !prefix.is_empty() =>
        {
            Invocation::Plugin(Management::BindProject { id, prefix })
        }
        [_, action] if action == OsStr::new("approve-project") => {
            Invocation::Plugin(Management::ApproveProject {
                expected_sha256: None,
            })
        }
        [_, action, sha, digest]
            if action == OsStr::new("approve-project") && sha == OsStr::new("--sha256") =>
        {
            Invocation::Plugin(Management::ApproveProject {
                expected_sha256: Some(digest),
            })
        }
        [_, action, id] if action == OsStr::new("pin") => {
            Invocation::Plugin(Management::Pin { id, sha256: None })
        }
        [_, action, id, sha, digest]
            if action == OsStr::new("pin") && sha == OsStr::new("--sha256") =>
        {
            Invocation::Plugin(Management::Pin {
                id,
                sha256: Some(digest),
            })
        }
        [_, action, id] if action == OsStr::new("unpin") => {
            Invocation::Plugin(Management::Unpin { id })
        }
        [_, action, id] if action == OsStr::new("untrust") => {
            Invocation::Plugin(Management::Untrust { id })
        }
        [_, action] if action == OsStr::new("revoke-project") => {
            Invocation::Plugin(Management::RevokeProject)
        }
        [_, action] if action == OsStr::new("list") => {
            Invocation::Plugin(Management::List { json: false })
        }
        [_, action, option] if action == OsStr::new("list") && option == OsStr::new("--json") => {
            Invocation::Plugin(Management::List { json: true })
        }
        [_, action, separator, argv @ ..]
            if action == OsStr::new("resolve")
                && separator == OsStr::new("--")
                && !argv.is_empty() =>
        {
            Invocation::Plugin(Management::Resolve { argv, json: false })
        }
        [_, action, option, separator, argv @ ..]
            if action == OsStr::new("resolve")
                && option == OsStr::new("--json")
                && separator == OsStr::new("--")
                && !argv.is_empty() =>
        {
            Invocation::Plugin(Management::Resolve { argv, json: true })
        }
        _ => Invocation::UsageError(b"invalid --plugin arguments"),
    }
}

fn parse_trust_flags(flags: &[OsString]) -> Option<(bool, bool, Option<&OsStr>, &OsStr)> {
    let mut index = 0;
    let mut pinned = false;
    let mut replace = false;
    let mut expected_sha256: Option<&OsStr> = None;
    while let Some(flag) = flags.get(index) {
        if flag == OsStr::new("--") {
            let path = flags.get(index + 1)?;
            if flags.get(index + 2).is_some() {
                return None;
            }
            return Some((pinned, replace, expected_sha256, path));
        }
        match flag.as_encoded_bytes() {
            b"--pin" => {
                if pinned {
                    return None;
                }
                pinned = true;
            }
            b"--replace" => {
                if replace {
                    return None;
                }
                replace = true;
            }
            b"--sha256" => {
                if expected_sha256.is_some() {
                    return None;
                }
                index += 1;
                expected_sha256 = Some(flags.get(index)?);
            }
            _ => return None,
        }
        index += 1;
    }
    None
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
    let target = Target::parse_bytes(target)?;
    let mut dry_run = false;
    let mut force = false;
    for option in &args[option_start..] {
        match option.as_encoded_bytes() {
            b"--dry-run" if !dry_run => dry_run = true,
            b"--force" if !force => force = true,
            _ => return None,
        }
    }
    SetupRequest::new(action, target, dry_run, force).ok()
}

fn is_deferred_mode(argument: &OsStr) -> bool {
    ["--stats", "--discover", "--err", "--test"]
        .iter()
        .any(|deferred| argument == OsStr::new(deferred))
}
