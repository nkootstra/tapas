# Security

## Supported versions

Until Tapas reaches a stable release, only the latest `0.x` release receives security fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through the GitHub Security Advisory flow for `nkootstra/tapas`. Do not open a public issue for an unpatched vulnerability or include secrets, private command output, or production configuration in a report.

Include the affected version and platform, the smallest safe reproduction, the security impact, and any suggested mitigation. Reports involving command execution, output corruption, setup ownership, symlink or permission handling, and release artifact integrity are especially useful.

## Security boundaries

Tapas executes the argv supplied by the caller directly and does not introduce a shell. It makes no telemetry or runtime network calls. Claude hook evaluation can deny an eligible noisy command and provide wrapping guidance, but it never grants command authority.

Process-filter plugins are output processors, not sandboxes. A trusted plugin is an arbitrary local process with the privileges of the Tapas user: it can read or modify accessible files, start other processes, and make network calls even though the Tapas runtime itself does not. Tapas invokes plugin paths directly without a shell and fails open to the original command output when a plugin fails, declines its selected route, violates limits, or returns an invalid response.

Path trust follows changes made to the trusted executable path. SHA-256 pinning rejects changed bytes until the plugin is deliberately updated or unpinned. Repository bindings require approval of the exact `.tapas.json`; approval says only that the routing configuration was reviewed, not that the plugin is safe. Protocol conformance likewise establishes neither trust, safety, nor semantic quality.

Pull-request and CI artifacts contain untrusted code. Checksums protect transfer integrity; they do not make a PR build trusted. Stable installation links and agent setup should never be replaced automatically by a test artifact.
