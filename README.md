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

Each Actions artifact contains `tapas`, `SHA256SUMS`, and `BUILD-METADATA.json` at its root.

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
