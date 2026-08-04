# Tapas

Tapas reduces the tokens consumed by command-line output before that output reaches a coding agent. It is the Rust successor to [smll](https://github.com/nkootstra/smll), with smll `v1.9.0` pinned as its behavioral oracle.

`0.1.0` is an internal compatibility release. It starts a fresh version line and intentionally has no `smll` executable alias, `SMLL_` environment aliases, or automatic `~/.smll` migration.

## Results

The pinned 94-case smll CLI corpus contains representative Git, test, build, package, listing, infrastructure, and log output. Using the vendored `o200k_base` tokenizer proxy, Tapas currently reduces that corpus from 509,798 to 159,524 tokens: 68.71% fewer tokens. Ninety-one combined outputs match the pinned smll baseline exactly; the remaining three cases are documented intentional differences, with an 18-token net increase versus the baseline. The two verbose-curl cases intentionally keep response bodies on stdout and compact request metadata on stderr; they preserve the pinned facts and token counts while correcting smll's stream routing.

This is a deterministic regression benchmark, not a claim about every model, prompt, or billing tokenizer. Tapas keeps exit status and actionable facts as the primary compatibility gates.

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
- Windows x86_64 MSVC (`x86_64-pc-windows-msvc`)

Each Actions artifact contains the platform binary, `SHA256SUMS`, and `BUILD-METADATA.json` at its root.

## Install releases and PR builds

Stable releases are published for Apple Silicon macOS, Linux x86_64 musl, Linux arm64 musl, and Windows x86_64. Install the latest stable release on Unix with:

```sh
curl -fsSL https://github.com/nkootstra/tapas/raw/refs/heads/main/install.sh | sh
```

On Windows PowerShell, install the latest stable release with:

```powershell
$script = Invoke-RestMethod 'https://github.com/nkootstra/tapas/raw/refs/heads/main/install.ps1'
& ([scriptblock]::Create($script))
```

Use `-Version 0.2.0` to install a specific tagged release or `-InstallDir C:\path\to\bin` to choose the destination directory.

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

The installer verifies the release checksum and source commit, then places the development binary at `~/.local/bin/tapas-pr-<commit>`. A PR build reports a version such as `tapas 0.1.0-dev.332d7176` so it is distinguishable from a stable build. PR builds are temporary and are removed remotely when the pull request is merged or after the retention window.

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
python3 scripts/parity.py --binary target/debug/tapas --tool tapas
```

Run the token benchmark:

```sh
python3 -m venv .benchmark-venv
.benchmark-venv/bin/pip install -r scripts/requirements-benchmark.txt
.benchmark-venv/bin/python scripts/historical_benchmark.py \
  --tapas-bin target/debug/tapas
```

The source inventory, fixtures, benchmark cases, tokenizer asset, and smll output baseline are pinned and hash-audited. The broader source inventory—not the historical benchmark alone—defines `0.1.0` command coverage.

## Current scope

`0.1.0` covers command, pipe, process, streaming, and user-level hook behavior for Claude and Codex. Stats, history, discovery, failure tee storage, and other agent integrations are intentionally deferred to later Tapas versions.

## License

Apache-2.0.
