use std::ffi::OsStr;
use std::path::Path;

/// The tapas command catalog.
///
/// Tapas owns this catalog directly. It is no longer derived from the archived
/// smll project; new commands are added here together with their filter
/// implementation, fixtures, and regression cases (enforced by
/// `scripts/audit_catalog.py`).
pub const AUTO_WRAP_COMMANDS: &[&str] = &[
    "acli",
    "aws",
    "bat",
    "batcat",
    "biome",
    "brew",
    "bun",
    "bunx",
    "cargo",
    "cat",
    "cmake",
    "composer",
    "ctest",
    "curl",
    "df",
    "docker",
    "docker-compose",
    "du",
    "esbuild",
    "eslint",
    "find",
    "gh",
    "git",
    "go",
    "gradle",
    "gradlew",
    "grep",
    "helm",
    "jest",
    "jq",
    "kubectl",
    "ls",
    "lsof",
    "make",
    "mocha",
    "mvn",
    "mvnw",
    "next",
    "ninja",
    "npm",
    "pip",
    "pip3",
    "playwright",
    "pnpm",
    "pre-commit",
    "ps",
    "psql",
    "pup",
    "pytest",
    "rg",
    "sqlite3",
    "systemctl",
    "terraform",
    "tofu",
    "tree",
    "tsc",
    "uv",
    "uvx",
    "vite",
    "vitest",
    "webpack",
    "yarn",
    "zig",
];
pub const WRAPPER_COMMANDS: &[&str] = &[
    "acli",
    "aws",
    "bash",
    "bat",
    "batcat",
    "biome",
    "brew",
    "bun",
    "bunx",
    "cargo",
    "cat",
    "cmake",
    "composer",
    "ctest",
    "curl",
    "df",
    "docker",
    "docker-compose",
    "dotnet",
    "du",
    "env",
    "esbuild",
    "eslint",
    "find",
    "gh",
    "git",
    "go",
    "gradle",
    "gradlew",
    "grep",
    "head",
    "helm",
    "jest",
    "jq",
    "kubectl",
    "ls",
    "lsof",
    "make",
    "mocha",
    "mvn",
    "mvnw",
    "mypy",
    "next",
    "ninja",
    "node",
    "npm",
    "pip",
    "pip3",
    "playwright",
    "pnpm",
    "pre-commit",
    "prettier",
    "ps",
    "psql",
    "pup",
    "pytest",
    "rg",
    "ruff",
    "sh",
    "sqlite3",
    "swift",
    "systemctl",
    "tail",
    "terraform",
    "tofu",
    "tree",
    "tsc",
    "turbo",
    "uv",
    "uvx",
    "vite",
    "vitest",
    "wc",
    "webpack",
    "xcodebuild",
    "yarn",
    "zig",
    "zsh",
];
pub const GIT_SUBCOMMANDS: &[&str] = &[
    "add", "blame", "branch", "checkout", "commit", "config", "diff", "fetch", "grep", "log",
    "merge", "pull", "push", "remote", "rebase", "reflog", "shortlog", "show", "stash", "status",
    "switch", "tag", "worktree",
];
pub const PIPE_DETECTORS: &[&str] = &[
    "git_status",
    "git_branch",
    "git_reflog",
    "git_show",
    "GitDiffPipe",
    "GitLogCompact",
    "git_commit",
    "git_merge",
    "git_blame",
    "cargo_test",
    "jest",
    "js_test",
    "tsc",
    "go_test",
    "pytest",
    "kubectl_compact",
    "docker_compact",
    "npm_install",
    "tree",
    "ls_compact",
    "FindCompactPipe",
    "DuCompactPipe",
    "CurlCompactPipe",
    "GenericCompactPipe",
];
pub const TRANSPARENT_RUNNERS: &[&str] =
    &["bunx", "npx", "pnpm exec", "poetry run", "uv run", "uvx"];
pub const EXACT_OUTPUT_BYPASSES: &[&str] = &[
    "ambiguous_runner",
    "find_exact_output",
    "git_alternate_format",
    "lossless_or_raw",
    "ls_exact_output",
    "machine_output",
    "query",
    "tree_exact_output",
];
pub const STREAM_WATCH_POLICIES: &[&str] = &[
    "bat_forced_paging_inherit",
    "ctest_repeat_inherit",
    "docker_compose_up_inherit",
    "docker_compose_logs_follow",
    "docker_logs_follow",
    "docker_run_inherit",
    "docker_stats_stream_inherit",
    "esbuild_watch_serve_inherit",
    "gh_run_watch",
    "jest_watch",
    "journalctl_follow",
    "kubectl_logs_follow",
    "playwright_interactive_inherit",
    "tail_follow",
    "tsc_watch",
    "unsupported_watch_inherit",
    "vite_lifecycle_inherit",
    "vitest_watch",
];

// Coarse command ownership for argv-aware stream filters. Commands can belong
// to more than one family; the process registry preserves the dispatch order.
pub(crate) const GIT_FILTER_COMMANDS: &[&[u8]] = &[b"git"];
pub(crate) const TEST_TOOLS_FILTER_COMMANDS: &[&[u8]] = &[
    b"pytest",
    b"jest",
    b"vitest",
    b"mocha",
    b"ctest",
    b"playwright",
    b"tsc",
    b"cargo",
    b"go",
    b"node",
    b"npm",
    b"pnpm",
    b"yarn",
    b"bun",
];
pub(crate) const LISTING_FILTER_COMMANDS: &[&[u8]] = &[
    b"find", b"tree", b"ls", b"du", b"wc", b"env", b"rg", b"grep", b"bat", b"batcat",
];
pub(crate) const BUILD_FILTER_COMMANDS: &[&[u8]] = &[
    b"make",
    b"ninja",
    b"cargo",
    b"go",
    b"zig",
    b"npm",
    b"pnpm",
    b"yarn",
    b"bun",
    b"webpack",
    b"vite",
    b"esbuild",
    b"cmake",
    b"turbo",
    b"next",
    b"dotnet",
    b"gradle",
    b"gradlew",
    b"mvn",
    b"mvnw",
    b"swift",
    b"xcodebuild",
    b"uv",
    b"uvx",
    b"poetry",
    b"npx",
    b"bunx",
];
pub(crate) const PACKAGE_FILTER_COMMANDS: &[&[u8]] = &[
    b"npm",
    b"pnpm",
    b"yarn",
    b"bun",
    b"composer",
    b"pip",
    b"pip3",
];
pub(crate) const INFRA_FILTER_COMMANDS: &[&[u8]] = &[
    b"curl",
    b"docker",
    b"docker-compose",
    b"kubectl",
    b"helm",
    b"gh",
    b"acli",
];
pub(crate) const DATA_FILTER_COMMANDS: &[&[u8]] = &[
    b"aws",
    b"jq",
    b"pup",
    b"acli",
    b"gh",
    b"sqlite3",
    b"cat",
    b"docker",
    b"docker-compose",
    b"kubectl",
    b"ps",
    b"df",
    b"psql",
    b"systemctl",
    b"lsof",
    b"npm",
    b"pnpm",
    b"yarn",
    b"brew",
    b"bun",
];
pub(crate) const DIAGNOSTICS_FILTER_COMMANDS: &[&[u8]] = &[
    b"mypy",
    b"ruff",
    b"eslint",
    b"biome",
    b"pre-commit",
    b"prettier",
    b"terraform",
    b"tofu",
];
#[expect(dead_code, reason = "consumed by scripts/audit_catalog.py")]
pub(crate) const FILTER_FAMILY_EXEMPTIONS: &[&[u8]] =
    &[b"bash", b"sh", b"zsh", b"env", b"head", b"tail"];

pub fn command_basename(command: &OsStr) -> Option<&OsStr> {
    Path::new(command).file_name()
}

pub(crate) fn command_basename_bytes(command: &[u8]) -> &[u8] {
    command
        .iter()
        .rposition(|byte| matches!(byte, b'/' | b'\\'))
        .map_or(command, |separator| &command[separator + 1..])
}

pub(crate) fn filter_family_handles(argv: &[&[u8]], commands: &[&[u8]]) -> bool {
    argv.first()
        .is_some_and(|command| commands.contains(&command_basename_bytes(command)))
}

pub fn should_auto_wrap(command: &OsStr) -> bool {
    let Some(basename) = command_basename(command) else {
        return false;
    };
    AUTO_WRAP_COMMANDS
        .iter()
        .any(|candidate| basename == OsStr::new(candidate))
}
