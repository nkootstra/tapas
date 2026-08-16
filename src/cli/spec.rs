use std::ffi::OsStr;
use std::io::{self, Write};

use crate::setup::Target;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Mode {
    Help,
    Version,
    Filters,
    Plugin,
    Raw,
    Explain,
    Rewrite,
    Completions,
    HookEval,
    Setup,
    Unsetup,
}

#[derive(Clone, Copy)]
pub enum ValueSet {
    Shells,
    Targets,
    RawSeparator,
    PluginActions,
}

#[derive(Clone, Copy)]
pub enum Completion {
    None,
    Values(ValueSet),
    TargetOptions { force_for: Option<Target> },
}

pub struct Command {
    pub mode: Mode,
    pub names: &'static [&'static str],
    pub usage: &'static [&'static str],
    pub description: &'static str,
    pub show_in_options: bool,
    pub completion: Completion,
}

pub const COMMANDS: &[Command] = &[
    Command {
        mode: Mode::Help,
        names: &["-h", "--help"],
        usage: &[],
        description: "Show this help",
        show_in_options: true,
        completion: Completion::None,
    },
    Command {
        mode: Mode::Version,
        names: &["--version"],
        usage: &[],
        description: "Show the Tapas version",
        show_in_options: true,
        completion: Completion::None,
    },
    Command {
        mode: Mode::Filters,
        names: &["--filters"],
        usage: &[],
        description: "List the static compatibility catalogs",
        show_in_options: true,
        completion: Completion::None,
    },
    Command {
        mode: Mode::Plugin,
        names: &["--plugin"],
        usage: &[
            "tapas --plugin check -- <absolute-path>",
            "tapas --plugin test <id>",
            "tapas --plugin resolve [--json] -- <cmd...>",
            "tapas --plugin trust <id> [--pin] [--replace] [--sha256 <hex>] -- <absolute-path>",
            "tapas --plugin bind <--user|--project> <id> -- <prefix...>",
            "tapas --plugin pin <id> [--sha256 <hex>]",
            "tapas --plugin <unpin|untrust|test> <id>",
            "tapas --plugin approve-project [--sha256 <hex>]",
            "tapas --plugin revoke-project",
            "tapas --plugin list [--json]",
        ],
        description: "Manage and inspect process-filter plugins",
        show_in_options: true,
        completion: Completion::Values(ValueSet::PluginActions),
    },
    Command {
        mode: Mode::Raw,
        names: &["--raw"],
        usage: &["tapas --raw [--] <cmd...>", "<cmd> | tapas --raw"],
        description: "Run without compacting output",
        show_in_options: false,
        completion: Completion::Values(ValueSet::RawSeparator),
    },
    Command {
        mode: Mode::Explain,
        names: &["--explain"],
        usage: &["tapas --explain <cmd...>"],
        description: "Explain filter selection and reduction",
        show_in_options: false,
        completion: Completion::None,
    },
    Command {
        mode: Mode::Rewrite,
        names: &["--rewrite"],
        usage: &["tapas --rewrite <cmd...>"],
        description: "Print the rewritten command",
        show_in_options: false,
        completion: Completion::None,
    },
    Command {
        mode: Mode::Completions,
        names: &["--completions"],
        usage: &[
            "tapas --completions bash",
            "tapas --completions zsh",
            "tapas --completions fish",
        ],
        description: "Generate shell completions",
        show_in_options: true,
        completion: Completion::Values(ValueSet::Shells),
    },
    Command {
        mode: Mode::HookEval,
        names: &["--hook-eval"],
        usage: &[
            "tapas --hook-eval claude",
            "tapas --hook-eval codex",
            "tapas --hook-eval opencode",
        ],
        description: "Evaluate an agent hook",
        show_in_options: false,
        completion: Completion::Values(ValueSet::Targets),
    },
    Command {
        mode: Mode::Setup,
        names: &["--setup"],
        usage: &[
            "tapas --setup claude [--dry-run]",
            "tapas --setup codex [--dry-run]",
            "tapas --setup opencode [--dry-run] [--force]",
        ],
        description: "Install an agent integration",
        show_in_options: false,
        completion: Completion::TargetOptions {
            force_for: Some(Target::OpenCode),
        },
    },
    Command {
        mode: Mode::Unsetup,
        names: &["--unsetup"],
        usage: &[
            "tapas --unsetup claude [--dry-run]",
            "tapas --unsetup codex [--dry-run]",
            "tapas --unsetup opencode [--dry-run]",
        ],
        description: "Remove an agent integration",
        show_in_options: false,
        completion: Completion::TargetOptions { force_for: None },
    },
];

impl Mode {
    pub fn parse(argument: &OsStr) -> Option<Self> {
        let bytes = argument.as_encoded_bytes();
        COMMANDS.iter().find_map(|command| {
            let matches_name = command.names.iter().any(|name| bytes == name.as_bytes());
            let matches_attached = matches!(command.mode, Mode::Setup | Mode::Unsetup)
                && command.names.iter().any(|name| {
                    bytes
                        .strip_prefix(name.as_bytes())
                        .is_some_and(|suffix| suffix.starts_with(b"="))
                });
            (matches_name || matches_attached).then_some(command.mode)
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub const ALL: [Self; 3] = [Self::Bash, Self::Zsh, Self::Fish];

    pub fn parse(value: &OsStr) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|shell| value.as_encoded_bytes() == shell.name().as_bytes())
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }
}

pub fn write_help(output: &mut dyn Write) -> io::Result<()> {
    output.write_all(b"Usage:\n  tapas <cmd...>\n  <cmd> | tapas\n")?;
    for command in COMMANDS {
        for usage in command.usage {
            writeln!(output, "  {usage}")?;
        }
    }
    output.write_all(b"\nOptions:\n")?;
    for command in COMMANDS.iter().filter(|command| command.show_in_options) {
        writeln!(
            output,
            "  {:<17}{}",
            command.names.join(", "),
            command.description
        )?;
    }
    Ok(())
}
