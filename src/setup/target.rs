use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Target {
    Claude,
    Codex,
    OpenCode,
}

impl Target {
    pub(crate) const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::OpenCode];

    pub fn parse(value: &OsStr) -> Option<Self> {
        Self::parse_bytes(value.as_encoded_bytes())
    }

    pub(crate) fn parse_bytes(value: &[u8]) -> Option<Self> {
        match value {
            b"claude" => Some(Self::Claude),
            b"codex" => Some(Self::Codex),
            b"opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    pub(super) const fn config_name(self) -> &'static str {
        match self {
            Self::Claude => "settings.json",
            Self::Codex => "hooks.json",
            Self::OpenCode => "tapas.js",
        }
    }

    pub(super) const fn supports_force(self) -> bool {
        matches!(self, Self::OpenCode)
    }

    pub(super) const fn grants_rewrite_permission(self) -> bool {
        matches!(self, Self::Codex)
    }

    pub(super) fn install_path(
        self,
        home: &Path,
        codex_home: Option<&Path>,
        xdg_config_home: Option<&Path>,
    ) -> PathBuf {
        match self {
            Self::Claude => home.join(".claude").join(self.config_name()),
            Self::Codex => codex_home
                .map_or_else(|| home.join(".codex"), Path::to_path_buf)
                .join(self.config_name()),
            Self::OpenCode => xdg_config_home
                .map_or_else(|| home.join(".config"), Path::to_path_buf)
                .join("opencode/plugins")
                .join(self.config_name()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::Target;

    #[test]
    fn target_install_paths_preserve_harness_conventions() {
        let home = Path::new("/home/tester");
        assert_eq!(
            Target::Claude.install_path(home, None, None),
            Path::new("/home/tester/.claude/settings.json")
        );
        assert_eq!(
            Target::Codex.install_path(home, None, None),
            Path::new("/home/tester/.codex/hooks.json")
        );
        assert_eq!(
            Target::Codex.install_path(home, Some(Path::new("/client/codex")), None),
            Path::new("/client/codex/hooks.json")
        );
        assert_eq!(
            Target::OpenCode.install_path(home, None, None),
            Path::new("/home/tester/.config/opencode/plugins/tapas.js")
        );
        assert_eq!(
            Target::OpenCode.install_path(home, None, Some(Path::new("/client/config"))),
            Path::new("/client/config/opencode/plugins/tapas.js")
        );
    }
}
