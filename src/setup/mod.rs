mod json;

use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicU64;

use json::Value;

const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;
const OWNERSHIP_HEADER: &[u8] = b"tapas-setup-v3\n";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Action {
    Setup,
    Unsetup,
}

pub use evaluator::{hook_eval, hook_eval_for_target};
pub use target::Target;

pub fn configure(
    action: Action,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    configure_for_target(action, Target::Claude, dry_run, stdout, stderr)
}

pub fn configure_for_target(
    action: Action,
    target: Target,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    configure_for_target_with_force(action, target, dry_run, false, stdout, stderr)
}

pub fn configure_for_target_with_force(
    action: Action,
    target: Target,
    dry_run: bool,
    force: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let request = match SetupRequest::new(action, target, dry_run, force) {
        Ok(request) => request,
        Err(InvalidSetupRequest::UnsupportedForce) => {
            stderr
                .write_all(b"tapas agent setup: --force is supported only for OpenCode setup\n")?;
            return Ok(2);
        }
    };
    configure_request(request, stdout, stderr)
}

pub(crate) fn configure_request(
    request: SetupRequest,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let context = match SetupContext::from_process(request)? {
        ContextResolution::Ready(context) => context,
        ContextResolution::MissingHome => {
            stderr.write_all(b"tapas agent setup: HOME is not set\n")?;
            return Ok(1);
        }
        ContextResolution::LegacyCodexPath => {
            stderr.write_all(b"legacy Codex ownership does not record its installation path; rerun `tapas --setup codex` with the original CODEX_HOME before unsetup\n")?;
            return Ok(1);
        }
    };
    configure_at(
        &context.location,
        &context.executable,
        context.request.action,
        context.request.dry_run,
        context.request.force,
        stdout,
        stderr,
    )
}

fn configure_at(
    location: &SetupLocation,
    executable: &Path,
    action: Action,
    dry_run: bool,
    force: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    if location.target == Target::OpenCode {
        configure_opencode(location, executable, action, dry_run, force, stdout, stderr)
    } else {
        configure_hook_installation(location, executable, action, dry_run, stdout, stderr)
    }
}

mod context;
mod evaluator;
mod hook_installation;
mod hooks;
mod lossless;
mod opencode;
mod ownership;
mod storage;
mod target;
mod transaction;
#[cfg(test)]
mod transaction_tests;

pub(crate) use context::SetupRequest;
use context::{ContextResolution, InvalidSetupRequest, SetupContext, SetupLocation};

use hook_installation::configure as configure_hook_installation;
#[cfg(test)]
use hooks::{eligible, ensure_hook, hook_entry, nested_hook_exists, remove_hook};
use opencode::configure_opencode;
use storage::{read_optional, write_atomic};

#[cfg(test)]
mod tests {
    use super::{Value, eligible, ensure_hook, hook_entry, remove_hook};

    #[test]
    fn hook_eligibility_accepts_simple_commands_and_rejects_shell_authority() {
        for command in [
            b"git status".as_slice(),
            b"'/usr/bin/git' diff",
            b"npm test -- --runInBand",
        ] {
            assert!(eligible(command), "{command:?}");
        }
        for command in [
            b"git status | tee out".as_slice(),
            b"git $(cat command)",
            b"git status\nrm -rf x",
            b"unknown command",
            b"\"git status",
        ] {
            assert!(!eligible(command), "{command:?}");
        }
    }

    #[test]
    fn hook_mutation_preserves_unrelated_entries_and_removes_only_owned_entry() {
        let other = Value::Object(vec![(
            b"command".to_vec(),
            Value::String(b"other-hook".to_vec()),
        )]);
        let mut root = Value::Object(vec![
            (b"theme".to_vec(), Value::String(b"dark".to_vec())),
            (
                b"hooks".to_vec(),
                Value::Object(vec![(
                    b"PreToolUse".to_vec(),
                    Value::Array(vec![Value::Object(vec![(
                        b"hooks".to_vec(),
                        Value::Array(vec![other]),
                    )])]),
                )]),
            ),
        ]);
        assert!(!ensure_hook(&mut root, b"tapas-hook").unwrap());
        assert!(ensure_hook(&mut root, b"tapas-hook").unwrap());
        assert!(remove_hook(&mut root, &hook_entry(b"tapas-hook")).unwrap());
        assert_eq!(root.get(b"theme"), Some(&Value::String(b"dark".to_vec())));
        let Value::Array(entries) = root
            .get(b"hooks")
            .and_then(|hooks| hooks.get(b"PreToolUse"))
            .expect("PreToolUse handlers")
        else {
            panic!("PreToolUse is not an array");
        };
        assert!(super::nested_hook_exists(entries, b"other-hook"));
    }
}
