use super::exact::trim_ascii;
use super::find_subslice;

#[derive(Clone, Copy)]
pub(super) enum Route {
    Status,
    Diff,
    Deploy,
    Resolve,
}

pub(super) fn route(argv: &[&[u8]]) -> Option<Route> {
    let mut index = 1;
    while index < argv.len() && argv[index] != b"migrate" {
        index += option_len(argv, index, &[b"--schema", b"--config"])?;
    }
    if argv.get(index).copied()? != b"migrate" {
        return None;
    }
    let subcommand = argv.get(index + 1).copied()?;
    let arguments = &argv[index + 2..];
    match subcommand {
        b"status" if consume_options(arguments, &[b"--schema", b"--config"], &[]) => {
            Some(Route::Status)
        }
        b"deploy" if consume_options(arguments, &[b"--schema", b"--config"], &[]) => {
            Some(Route::Deploy)
        }
        b"resolve"
            if consume_options(
                arguments,
                &[b"--schema", b"--config", b"--applied", b"--rolled-back"],
                &[],
            ) && resolve_action_count(arguments) == 1 =>
        {
            Some(Route::Resolve)
        }
        b"diff"
            if consume_options(
                arguments,
                &[
                    b"--from-url",
                    b"--to-url",
                    b"--from-schema",
                    b"--to-schema",
                    b"--from-schema-datamodel",
                    b"--to-schema-datamodel",
                    b"--from-migrations",
                    b"--to-migrations",
                    b"--shadow-database-url",
                ],
                &[
                    b"--from-empty",
                    b"--to-empty",
                    b"--from-local-d1",
                    b"--to-local-d1",
                    b"--exit-code",
                ],
            ) && has_diff_side(arguments, b"--from-")
                && has_diff_side(arguments, b"--to-") =>
        {
            Some(Route::Diff)
        }
        _ => None,
    }
}

fn resolve_action_count(arguments: &[&[u8]]) -> usize {
    arguments
        .iter()
        .filter(|argument| {
            matches!(**argument, b"--applied" | b"--rolled-back")
                || argument.starts_with(b"--applied=")
                || argument.starts_with(b"--rolled-back=")
        })
        .count()
}

fn has_diff_side(arguments: &[&[u8]], prefix: &[u8]) -> bool {
    arguments
        .iter()
        .any(|argument| argument.starts_with(prefix))
}

fn consume_options(arguments: &[&[u8]], values: &[&[u8]], switches: &[&[u8]]) -> bool {
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == b"--" {
            return false;
        }
        let Some(len) = option_len(arguments, index, values) else {
            if switches.contains(&arguments[index]) {
                index += 1;
                continue;
            }
            return false;
        };
        index += len;
    }
    true
}

fn option_len(arguments: &[&[u8]], index: usize, values: &[&[u8]]) -> Option<usize> {
    let argument = *arguments.get(index)?;
    for option in values {
        if argument == *option {
            return arguments
                .get(index + 1)
                .filter(|value| !value.is_empty())
                .map(|_| 2);
        }
        if argument
            .strip_prefix(*option)
            .is_some_and(|rest| rest.starts_with(b"=") && rest.len() > 1)
        {
            return Some(1);
        }
    }
    None
}

pub(super) fn matches(route: Route, input: &[u8]) -> bool {
    input.split(|byte| *byte == b'\n').any(|raw| {
        let line = trim_ascii(raw);
        is_error(line)
            || match route {
                Route::Status => {
                    find_subslice(line, b"migration found").is_some()
                        || find_subslice(line, b"migrations found").is_some()
                        || find_subslice(line, b"migration have not yet been applied").is_some()
                        || find_subslice(line, b"migration is not yet applied").is_some()
                        || find_subslice(line, b"Database schema is up to date").is_some()
                        || find_subslice(line, b"database schema is not in sync").is_some()
                }
                Route::Diff => {
                    matches!(line.first(), Some(b'+' | b'-' | b'*'))
                        || line.starts_with(b"[+]")
                        || line.starts_with(b"[-]")
                        || line.starts_with(b"[*]")
                        || line.starts_with(b"No difference detected")
                        || line.starts_with(b"No difference was detected")
                }
                Route::Deploy => {
                    line.starts_with(b"Applying migration ")
                        || find_subslice(line, b"migration(s) have been applied").is_some()
                        || line.starts_with(b"No pending migrations")
                }
                Route::Resolve => {
                    line.starts_with(b"Migration ")
                        && (line.ends_with(b" marked as applied.")
                            || line.ends_with(b" marked as rolled back."))
                }
            }
    })
}

fn is_error(line: &[u8]) -> bool {
    line.starts_with(b"Error:")
        || line.starts_with(b"ERROR ")
        || line.starts_with(b"P3")
        || find_subslice(line, b"failed migration").is_some()
        || find_subslice(line, b"migration failed").is_some()
}

pub(super) fn compact(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for input in [stdout, stderr] {
        let mut dropping_prelude = false;
        for raw in input.split_inclusive(|byte| *byte == b'\n') {
            let line = raw.strip_suffix(b"\n").unwrap_or(raw);
            if is_prelude(trim_ascii(line)) {
                dropping_prelude = true;
                continue;
            }
            if dropping_prelude && trim_ascii(line).is_empty() {
                continue;
            }
            dropping_prelude = false;
            output.extend_from_slice(raw);
        }
    }
    output
}

fn is_prelude(line: &[u8]) -> bool {
    line.starts_with(b"Environment variables loaded from ")
        || line.starts_with(b"Prisma schema loaded from ")
        || line.starts_with(b"Datasource \"")
}
