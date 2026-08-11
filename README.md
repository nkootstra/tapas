# Tapas

Tapas reduces the tokens consumed by command-line output before that output reaches a coding agent. It is the maintained Rust successor to the archived [smll](https://github.com/nkootstra/smll) project, and it owns its command catalog and regression corpus directly.

`0.3.0` continues the Tapas version line and intentionally has no `smll` executable alias, `SMLL_` environment aliases, or automatic `~/.smll` migration.

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

The default catalog includes package workflows (`pip`, `pip3`, `uv`, and
`uvx`), frontend and native builds (`vite`, `esbuild`, `cmake`, and `ctest`),
Playwright tests, Helm reads, multi-file human `grep` output, plain `bat` and
`batcat` output, and recognized Docker BuildKit and finite stats output.
`tapas --filters` reports the compact routes as well as exact-output and
inherited/stream policies from the same catalog used at runtime.

These routes are deliberately conservative. Machine formats, custom or
ambiguous output, malformed data, and unsupported option combinations remain
byte-exact. Interactive, watched, paged, or otherwise unbounded commands
inherit the terminal instead of being buffered.

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

Invalid Tapas options print an explanation followed by the complete help text. Options that appear after a wrapped command belong to that command and pass through unchanged.

## Shell completions

Tapas generates completions for its own options, modes, setup targets, and shell names. It stops offering Tapas-specific candidates once a wrapped command begins and never edits shell configuration automatically.

Load completions for the current shell session:

```sh
# Bash
source <(tapas --completions bash)

# Zsh (after compinit)
source <(tapas --completions zsh)

# Fish
tapas --completions fish | source
```

To enable them permanently, add the relevant command to the shell's startup file. Fish users can instead save the generated script at `~/.config/fish/completions/tapas.fish`.

## Agent setup

Install a user-level integration for a supported coding agent:

```sh
tapas --setup claude
tapas --setup codex
tapas --setup opencode
```

Preview or remove it:

```sh
tapas --setup claude --dry-run
tapas --unsetup claude
tapas --setup codex --dry-run
tapas --unsetup codex
tapas --setup opencode --dry-run
tapas --unsetup opencode
```

Custom Codex homes are supported. Ownership records the absolute hook path, so removal still targets the installed profile if `CODEX_HOME` later changes. Running setup with a different `CODEX_HOME` safely relocates an unmodified Tapas-owned hook:

```sh
CODEX_HOME=/path/to/codex-home tapas --setup codex
CODEX_HOME=/another/codex-home tapas --setup codex
```

Setup preserves the original bytes, ordering, formatting, number spellings, escapes, unrelated settings, and unrelated handlers in Claude and Codex JSON files. It rejects ambiguous JSON, symbolic links, non-regular managed files, duplicate keys, and unowned Tapas-looking hooks. Changes are written atomically with non-overwriting private backups and path-bound ownership under `~/.tapas/setup`. Unsetup restores the exact pre-install file when it is unchanged apart from Tapas; after unrelated user edits it removes only the uniquely owned hook.

Running `--setup codex` from a development build points Codex at that exact executable. If it replaces a different Tapas-owned hook, Tapas prints a warning naming the active development version and executable path.

The Claude integration writes `~/.claude/settings.json` and provides rewrite guidance without granting command permission. The Codex integration writes `${CODEX_HOME:-$HOME/.codex}/hooks.json`; after setup, open `/hooks` to review and trust the exact hook that was added. Review other matching hooks from every active user, project, profile, and plugin layer at the same time. Codex requires an allow decision when a hook supplies updated input, so Tapas limits Codex rewrites to unqualified, local read-only command forms resolved through absolute executable search paths outside the session workspace. The executable and all its path ancestors must not be group- or world-writable; unsafe candidates are skipped in favor of a later trusted candidate. Tapas disables supported tools' configuration-driven helper execution before running them. Mutating flags, commands that run project code, network tools, shell operators, substitutions, multiline commands, unsupported commands, and already wrapped commands are left untouched.

Claude and OpenCode can recognize supported commands through as many as four
declared transparent runner layers (for example `npx`, `pnpm exec`, `uv run`,
or `uvx`). Ambiguous chains and a fifth runner layer are left unchanged.
Codex keeps its narrower read-only allowlist.

The OpenCode integration installs a dependency-free stable V1 plugin at `${XDG_CONFIG_HOME:-$HOME/.config}/opencode/plugins/tapas.js`. The plugin invokes the absolute Tapas executable without a shell, changes only eligible Bash command text, preserves every other tool argument, and fails open to the original command if evaluation fails. OpenCode V2 beta is not supported.

Tapas detects recognized `smll` and `rtk` OpenCode integrations before installation. Without `--force`, setup warns and changes nothing. `tapas --setup opencode --force` removes only recognized user-level OpenCode plugin files, exact strict-JSON registration entries, and matching OpenCode ownership before installing Tapas in the same operation. Project, custom, inline, JSONC, modified, ambiguous, and symlinked conflicts remain hard blockers; Tapas never removes predecessor binaries, packages, caches, unrelated files, or non-empty directories. `--force` is intentionally invalid for Claude, Codex, and every unsetup command.

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

Use `--version 0.3.0` to install a specific tagged Unix release.

Stable releases are tag-driven. Update `Cargo.toml`, merge the version change, then create and push a matching tag:

```sh
git tag -s v0.3.0 -m "Release v0.3.0"
git push origin v0.3.0
```

The tag determines whether the release is a patch, minor, or major version; ordinary merges to `main` do not publish stable releases.

To test a pull request before it is merged, install the exact build for its current head commit:

```sh
curl -fsSL https://github.com/nkootstra/tapas/raw/refs/heads/main/install-pr.sh | sh -s -- 123
```

The installer verifies the release checksum and source commit, then places the development binary at `~/.local/bin/tapas-pr-<commit>`. A PR build reports a version such as `tapas 0.3.0-dev.332d7176` so it is distinguishable from a stable build. PR builds are temporary and are removed remotely when the pull request is merged or after the retention window.

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

Exercise supported tools against live output in disposable temporary projects:

```sh
cargo build --locked
python3 scripts/smoke-supported-commands.py \
  --binary target/debug/tapas \
  --require-all
```

The live smoke suite verifies that compact routes reduce output while retaining
named facts, and that machine-readable and failed-command routes remain
byte-exact. Tool groups whose dependencies are unavailable are skipped unless
`--require-all` is set. Docker and Helm cases require a running Docker daemon;
the Helm case creates and removes a temporary Kind cluster.

Run the pinned real-harness contracts with Node.js 22 or newer. These tests use
aimock locally and do not require provider credentials:

```sh
cargo build --locked
npm --prefix tests/harness-e2e ci
for harness in claude codex opencode; do
  TAPAS_HARNESS="$harness" npm --prefix tests/harness-e2e test
done
```

Inspect command usage against the catalog:

```sh
python3 scripts/usage_report.py --format json --minimum 5
```

The usage report reads session history without modifying it and highlights
commands or Git subcommands that are used but not covered by the catalog. Its
additive effective-command fields distinguish runtime routing coverage from a
declared compact route, and report transparent runner chains up to the same
four-layer runtime limit. A catalogued command without a declared compact
route is reported separately from an unlisted command.

The command catalog in `src/catalog.rs` is tapas-owned and audited for internal consistency: every auto-wrap command must be backed by a filter family and behavior test, every git subcommand must have a dispatch arm and behavior coverage, and every declared compact, exact-output, and inherited/stream policy must point to meaningful test or regression coverage. The regression corpus under `tests/regression/` is static test data, grown alongside new filters.

## Current scope

`0.3.0` covers command, pipe, process, streaming, and user-level integrations for Claude, Codex, and stable OpenCode V1. Stats, history, discovery, failure tee storage, and other agent integrations are intentionally deferred to later Tapas versions.

## License

Apache-2.0.
