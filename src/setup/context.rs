use std::io;
use std::path::PathBuf;

use super::ownership::{Ownership, read_ownership, recorded_path};
use super::{Action, Target};

pub(crate) struct SetupRequest {
    pub(super) action: Action,
    pub(super) target: Target,
    pub(super) dry_run: bool,
    pub(super) force: bool,
}

impl SetupRequest {
    pub(crate) fn new(
        action: Action,
        target: Target,
        dry_run: bool,
        force: bool,
    ) -> Result<Self, InvalidSetupRequest> {
        if force && (action != Action::Setup || !target.supports_force()) {
            return Err(InvalidSetupRequest::UnsupportedForce);
        }
        Ok(Self {
            action,
            target,
            dry_run,
            force,
        })
    }
}

pub(crate) enum InvalidSetupRequest {
    UnsupportedForce,
}

pub(super) struct SetupLocation {
    pub(super) config_path: PathBuf,
    pub(super) ownership_path: PathBuf,
    pub(super) target: Target,
}

pub(super) struct SetupContext {
    pub(super) request: SetupRequest,
    pub(super) location: SetupLocation,
    pub(super) executable: PathBuf,
}

pub(super) enum ContextResolution {
    Ready(SetupContext),
    MissingHome,
    LegacyCodexPath,
}

impl SetupContext {
    pub(super) fn from_process(request: SetupRequest) -> io::Result<ContextResolution> {
        let Some(home) = std::env::var_os("HOME") else {
            return Ok(ContextResolution::MissingHome);
        };
        let executable = std::env::current_exe()?;
        let (codex_home, xdg_config_home) = match request.target {
            Target::Claude => (None, None),
            Target::Codex => (nonempty_env("CODEX_HOME"), None),
            Target::OpenCode => (None, nonempty_env("XDG_CONFIG_HOME")),
        };
        let home = PathBuf::from(home);
        let ownership_path = home.join(format!(".tapas/setup/{}.owned", request.target.name()));
        let resolved_path =
            request
                .target
                .install_path(&home, codex_home.as_deref(), xdg_config_home.as_deref());
        let config_path = if request.action == Action::Unsetup {
            match read_ownership(&ownership_path)? {
                Ownership::Valid(value) => match recorded_path(&value) {
                    Some(path) => path,
                    None if request.target == Target::Codex => {
                        return Ok(ContextResolution::LegacyCodexPath);
                    }
                    None => resolved_path,
                },
                Ownership::Missing | Ownership::Modified => resolved_path,
            }
        } else {
            resolved_path
        };
        Ok(ContextResolution::Ready(Self {
            location: SetupLocation {
                config_path,
                ownership_path,
                target: request.target,
            },
            request,
            executable,
        }))
    }
}

fn nonempty_env(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{InvalidSetupRequest, SetupRequest};
    use crate::setup::{Action, Target};

    #[test]
    fn force_is_valid_only_for_opencode_setup() {
        assert!(SetupRequest::new(Action::Setup, Target::OpenCode, false, true).is_ok());
        assert!(matches!(
            SetupRequest::new(Action::Setup, Target::Claude, false, true),
            Err(InvalidSetupRequest::UnsupportedForce)
        ));
        assert!(matches!(
            SetupRequest::new(Action::Setup, Target::Codex, false, true),
            Err(InvalidSetupRequest::UnsupportedForce)
        ));
        assert!(matches!(
            SetupRequest::new(Action::Unsetup, Target::OpenCode, false, true),
            Err(InvalidSetupRequest::UnsupportedForce)
        ));
    }
}
