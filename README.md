# Tapas

Tapas reduces the tokens consumed by command-line output before that output reaches a coding agent. It is the maintained Rust successor to the archived [smll](https://github.com/nkootstra/smll) project, and it owns its command catalog and regression corpus directly.

`0.2.0` continues the Tapas version line and intentionally has no `smll` executable alias, `SMLL_` environment aliases, or automatic `~/.smll` migration.

## Results

Tapas is validated against a static regression corpus of representative Git, test, build, package, listing, infrastructure, and log output (`tests/regression/`). Every filter is required to keep exit status and actionable facts intact, and to leave the output readable for an agent; compaction that would drop a fact falls open to the raw output instead. Intentional divergences from the historical smll behavior are documented in `tests/regression/intentional-differences.json`.

This is a deterministic regression guarantee, not a claim about every model, prompt, or billing tokenizer. Exit status and actionable facts are the primary compatibility gates.

## Usage

Wrap a command:

```sh
tapas git status
tapas npm install
tapas cargo test
```

Filter existing output:

```sh
git status | tapas
```

Inspect the static command and runner catalogs:

```sh
tapas --filters
```

Force exact output when needed:

```sh
tapas --raw -- git diff
TAPAS_LOSSLESS=1 tapas git diff
```

Explain the selected filter and reduction for one command:

```sh
tapas --explain git status
```

Continuous logs and supported watch modes remain raw by default. Opt into bounded live compaction with:

```sh
TAPAS_STREAM=1 tapas docker logs --follow api
TAPAS_STREAM=1 tapas tsc --watch
```

Tapas never invokes a shell to run a wrapped command. Shell execution happens only when the caller explicitly requests a shell, such as `tapas sh -c '...'`.

## Agent setup

Install a user-level `PreToolUse` hook for either supported coding agent:

```sh
tapas --setup claude
tapas --setup codex
```

Preview or remove it:

```sh
tapas --setup claude --dry-run
tapas --unsetup claude
tapas --setup codex --dry-run
tapas --unsetup codex
```

If you use a custom client home, pass the same value during setup and removal:

```sh
CODEX_HOME=/path/to/codex-home tapas --setup codex
CODEX_HOME=/path/to/codex-home tapas --unsetup codex
```

Setup preserves unrelated settings and handlers, writes atomically, keeps a backup beside the client configuration file, and records private ownership under `~/.tapas/setup`. Unsetup removes only the exact hook recorded by Tapas.

Running `--setup codex` from a development build points Codex at that exact executable. If it replaces a different Tapas-owned hook, Tapas prints a warning naming the active development version and executable path.

The Claude integration writes `~/.claude/settings.json` and provides rewrite guidance without granting command permission. The Codex integration writes `${CODEX_HOME:-$HOME/.codex}/hooks.json`; after setup, open `/hooks` to review and trust the exact hook that was added. Review other matching hooks from every active user, project, profile, and plugin layer at the same time. Codex requires an allow decision when a hook supplies updated input, so Tapas limits Codex rewrites to unqualified, local read-only command forms resolved through absolute executable search paths outside the session workspace. The executable and all its path ancestors must not be group- or world-writable; unsafe candidates are skipped in favor of a later trusted candidate. Tapas disables supported tools' configuration-driven helper execution before running them. Mutating flags, commands that run project code, network tools, shell operators, substitutions, multiline commands, unsupported commands, and already wrapped commands are left untouched.

## Build

The toolchain is locked in `rust-toolchain.toml`:

```sh
cargo build --locked --profile z
cargo test --locked
```

The only runtime crate is `libc`, used at the Unix process boundary. Tapas makes no telemetry or network calls at runtime.

CI produces checksummed release-profile artifacts for:

- Apple Silicon macOS (`aarch64-apple-darwin`)
- Linux x86_64 musl (`x86_64-unknown-linux-musl`)
- Linux arm64 musl (`aarch64-unknown-linux-musl`)

Each Actions artifact contains the platform binary, `SHA256SUMS`, and `BUILD-METADATA.json` at its root.

## Install releases and PR builds

Stable releases are published for Apple Silicon macOS, Linux x86_64 musl, and Linux arm64 musl. Install the latest stable release on Unix with:

```sh
curl -fsSL https://github.com/nkootstra/tapas/raw/refs/heads/main/install.sh | sh
```

Windows stable binaries are not published yet because the current runtime uses Unix-specific process and filesystem APIs. The PowerShell script currently supports development-build cleanup only.

Use `--version 0.2.0` to install a specific tagged Unix release.

Stable releases are tag-driven. Update `Cargo.toml`, merge the version change, then create and push a matching tag:

```sh
git tag -s v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

The tag determines whether the release is a patch, minor, or major version; ordinary merges to `main` do not publish stable releases.

To test a pull request before it is merged, install the exact build for its current head commit:

```sh
curl -fsSL https://github.com/nkootstra/tapas/raw/refs/heads/main/install-pr.sh | sh -s -- 123
```

The installer verifies the release checksum and source commit, then places the development binary at `~/.local/bin/tapas-pr-<commit>`. A PR build reports a version such as `tapas 0.2.0-dev.332d7176` so it is distinguishable from a stable build. PR builds are temporary and are removed remotely when the pull request is merged or after the retention window.

Remove local PR builds without affecting the stable `tapas` executable:

```sh
curl -fsSL https://github.com/nkootstra/tapas/raw/refs/heads/main/install.sh | sh -s -- --clean-dev-builds --dry-run
curl -fsSL https://github.com/nkootstra/tapas/raw/refs/heads/main/install.sh | sh -s -- --clean-dev-builds
```

If the branch URL is temporarily cached and prints the old usage text, use a commit-pinned copy of `install.sh` from the latest `main` commit.

The PR workflow posts both the install and local cleanup commands to the pull request.

On Windows, run the PowerShell cleanup script as follows:

```powershell
$script = Invoke-RestMethod 'https://github.com/nkootstra/tapas/raw/refs/heads/main/install.ps1'
& ([scriptblock]::Create($script)) -CleanDevBuilds -DryRun
& ([scriptblock]::Create($script)) -CleanDevBuilds
```

Use `-InstallDir C:\path\to\bin` when development builds are stored outside the default directory. The script only removes `tapas-pr-*` files and leaves `tapas.exe` untouched.

## Verification

Run the standard gates:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover --start-directory tests/scripts --pattern 'test_*.py'
python3 scripts/audit_catalog.py
```

Run the regression parity harness against a built binary:

```sh
cargo build --locked
python3 scripts/parity.py --binary target/debug/tapas --tool tapas
```

Inspect command usage against the catalog:

```sh
python3 scripts/usage_report.py --format json --minimum 5
```

The usage report reads session history without modifying it and highlights
commands or Git subcommands that are used but not covered by the catalog.

The command catalog in `src/catalog.rs` is tapas-owned and audited for internal consistency: every auto-wrap command must be backed by a filter family, every git subcommand must have a dispatch arm, and every filter family must have regression tests. The regression corpus under `tests/regression/` is static test data, grown alongside new filters.

## Current scope

`0.2.0` covers command, pipe, process, streaming, and user-level hook behavior for Claude and Codex. Stats, history, discovery, failure tee storage, and other agent integrations are intentionally deferred to later Tapas versions.

## License

Apache-2.0.
