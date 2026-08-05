use std::io::{self, Write};

use crate::cli::spec::{self, Command, Completion, Shell};

pub fn write(shell: Shell, output: &mut dyn Write) -> io::Result<()> {
    match shell {
        Shell::Bash => write_bash(output),
        Shell::Zsh => write_zsh(output),
        Shell::Fish => write_fish(output),
    }
}

fn write_bash(output: &mut dyn Write) -> io::Result<()> {
    output.write_all(
        b"_tapas() {\n    local current first target candidates\n    COMPREPLY=()\n    current=\"${COMP_WORDS[COMP_CWORD]}\"\n    first=\"${COMP_WORDS[1]}\"\n\n    if (( COMP_CWORD == 1 )); then\n        COMPREPLY=( $(compgen -W '",
    )?;
    write_names(output, spec::COMMANDS.iter())?;
    output
        .write_all(b"' -- \"$current\") )\n        return\n    fi\n\n    case \"$first\" in\n")?;

    for command in spec::COMMANDS {
        match command.completion {
            Completion::None => {}
            Completion::Values(values) => {
                writeln!(output, "        {})", primary_name(command))?;
                output.write_all(b"            if (( COMP_CWORD == 2 )); then\n                COMPREPLY=( $(compgen -W '")?;
                write_values(output, values)?;
                output.write_all(b"' -- \"$current\") )\n            fi\n            ;;\n")?;
            }
            Completion::TargetOptions { force_for } => {
                writeln!(output, "        {})", primary_name(command))?;
                output.write_all(b"            if (( COMP_CWORD == 2 )); then\n                COMPREPLY=( $(compgen -W '")?;
                write_target_names(output)?;
                output.write_all(b"' -- \"$current\") )\n            else\n                target=\"${COMP_WORDS[2]}\"\n                candidates='")?;
                output.write_all(b"--dry-run'\n")?;
                if let Some(target) = force_for {
                    writeln!(
                        output,
                        "                [[ \"$target\" == {} ]] && candidates+=\" --force\"",
                        target.name()
                    )?;
                }
                output.write_all(b"                COMPREPLY=( $(compgen -W \"$candidates\" -- \"$current\") )\n            fi\n            ;;\n")?;
            }
        }
    }

    output.write_all(b"    esac\n}\n\ncomplete -F _tapas tapas\n")
}

fn write_zsh(output: &mut dyn Write) -> io::Result<()> {
    output.write_all(b"#compdef tapas\n\n_tapas() {\n    local first target\n    local -a top targets options\n    top=(")?;
    write_names(output, spec::COMMANDS.iter())?;
    output.write_all(b")\n    targets=(")?;
    write_target_names(output)?;
    output.write_all(b")\n\n    if (( CURRENT == 2 )); then\n        compadd -- $top\n        return\n    fi\n\n    first=\"$words[2]\"\n    case \"$first\" in\n")?;

    for command in spec::COMMANDS {
        match command.completion {
            Completion::None => {}
            Completion::Values(values) => {
                writeln!(output, "        {})", primary_name(command))?;
                output.write_all(b"            (( CURRENT == 3 )) && compadd -- ")?;
                write_values(output, values)?;
                output.write_all(b"\n            ;;\n")?;
            }
            Completion::TargetOptions { force_for } => {
                writeln!(output, "        {})", primary_name(command))?;
                output.write_all(b"            if (( CURRENT == 3 )); then\n                compadd -- $targets\n            else\n                target=\"$words[3]\"\n                options=(")?;
                output.write_all(b"--dry-run)\n")?;
                if let Some(target) = force_for {
                    writeln!(
                        output,
                        "                [[ \"$target\" == {} ]] && options+=(--force)",
                        target.name()
                    )?;
                }
                output.write_all(
                    b"                compadd -- $options\n            fi\n            ;;\n",
                )?;
            }
        }
    }

    output.write_all(b"    esac\n}\n\ncompdef _tapas tapas\n")
}

fn write_fish(output: &mut dyn Write) -> io::Result<()> {
    output.write_all(
        b"function __tapas_arg_count_is\n    test (count (commandline -opc)) -eq $argv[1]\nend\n\nfunction __tapas_first_arg_is\n    set -l words (commandline -opc)\n    test (count $words) -ge 2; and test \"$words[2]\" = \"$argv[1]\"\nend\n\nfunction __tapas_target_is\n    set -l words (commandline -opc)\n    test (count $words) -ge 3; and test \"$words[3]\" = \"$argv[1]\"\nend\n\n",
    )?;

    for command in spec::COMMANDS {
        output.write_all(b"complete -c tapas -n '__tapas_arg_count_is 1'")?;
        for name in command.names {
            if let Some(long) = name.strip_prefix("--") {
                write!(output, " -l {long}")?;
            } else if let Some(short) = name.strip_prefix('-') {
                write!(output, " -s {short}")?;
            }
        }
        writeln!(output, " -d '{}'", command.description)?;
    }
    output.write_all(b"\n")?;

    for command in spec::COMMANDS {
        let name = primary_name(command);
        match command.completion {
            Completion::None => {}
            Completion::Values(values) => {
                write!(
                    output,
                    "complete -c tapas -n '__tapas_first_arg_is {name}; and __tapas_arg_count_is 2' -a '"
                )?;
                write_values(output, values)?;
                output.write_all(b"'\n")?;
            }
            Completion::TargetOptions { force_for } => {
                write!(
                    output,
                    "complete -c tapas -n '__tapas_first_arg_is {name}; and __tapas_arg_count_is 2' -a '"
                )?;
                write_target_names(output)?;
                output.write_all(b"'\n")?;
                writeln!(
                    output,
                    "complete -c tapas -n '__tapas_first_arg_is {name}; and not __tapas_arg_count_is 2' -l dry-run"
                )?;
                if let Some(target) = force_for {
                    writeln!(
                        output,
                        "complete -c tapas -n '__tapas_first_arg_is {name}; and __tapas_target_is {}; and not __tapas_arg_count_is 2' -l force",
                        target.name()
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn primary_name(command: &Command) -> &'static str {
    command.names.last().copied().expect("command has a name")
}

fn write_names<'a>(
    output: &mut dyn Write,
    commands: impl Iterator<Item = &'a Command>,
) -> io::Result<()> {
    write_words(
        output,
        commands.flat_map(|command| command.names.iter().copied()),
    )
}

fn write_values(output: &mut dyn Write, values: spec::ValueSet) -> io::Result<()> {
    match values {
        spec::ValueSet::Shells => {
            write_words(output, spec::Shell::ALL.iter().map(|shell| shell.name()))
        }
        spec::ValueSet::Targets => write_target_names(output),
        spec::ValueSet::RawSeparator => output.write_all(b"--"),
    }
}

fn write_target_names(output: &mut dyn Write) -> io::Result<()> {
    write_words(
        output,
        crate::setup::Target::ALL.iter().map(|target| target.name()),
    )
}

fn write_words<'a>(output: &mut dyn Write, words: impl Iterator<Item = &'a str>) -> io::Result<()> {
    for (index, word) in words.enumerate() {
        if index != 0 {
            output.write_all(b" ")?;
        }
        output.write_all(word.as_bytes())?;
    }
    Ok(())
}
