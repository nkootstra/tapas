# Process-filter plugin examples

Tapas process-filter plugins are executable files that speak a dependency-free JSON-lines protocol. The executable writes one hello record, reads one request record, and writes one response record. Protocol v1 transports `argv`, the working directory, stdout, and stderr as base64 so paths and command output remain language-neutral and binary-safe. A response either transforms both streams and declares `fact-complete` or `potentially-lossy` evidence, or declines an unsupported command shape. The checked-in Node example shows that one executable can implement multiple reducers (`acme test` and `acme build`) by decoding and inspecting argv.

Use this author workflow:

```sh
# Check protocol behavior without changing trust state.
tapas --plugin check -- "$PWD/examples/plugins/node/acme-tools.mjs"

# Trust the executable path, or add --pin / --sha256 DIGEST.
tapas --plugin trust acme-tools -- "$PWD/examples/plugins/node/acme-tools.mjs"

# One plugin may be bound to multiple command prefixes.
tapas --plugin bind --user acme-tools -- acme test
tapas --plugin bind --project acme-tools -- acme build
tapas --plugin approve-project

tapas --plugin resolve --json -- acme test
tapas acme test
tapas --plugin test acme-tools

# Inspect and update trust deliberately.
tapas --plugin list --json
tapas --plugin pin acme-tools
tapas --plugin unpin acme-tools
tapas --plugin untrust acme-tools
```

User bindings are local. Project bindings live in `.tapas.json` and remain inactive until that exact repository configuration is approved. Path trust accepts updates to the executable at the trusted path; pinned trust requires the recorded SHA-256. Re-run `check`, inspect changes, and pin again when adopting a new version.

Plugins reduce decoded output bytes sent to agents. That is not a promise about exact tokenizer counts, which differ by model and tokenizer. The fixtures contain verbose inputs and exact compact outputs used by both examples and CI.
