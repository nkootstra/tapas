# Security

## Supported versions

Until Tapas reaches a stable release, only the latest `0.x` release receives security fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through the GitHub Security Advisory flow for `nkootstra/tapas`. Do not open a public issue for an unpatched vulnerability or include secrets, private command output, or production configuration in a report.

Include the affected version and platform, the smallest safe reproduction, the security impact, and any suggested mitigation. Reports involving command execution, output corruption, setup ownership, symlink or permission handling, and release artifact integrity are especially useful.

## Security boundaries

Tapas executes the argv supplied by the caller directly and does not introduce a shell. It makes no telemetry or runtime network calls. Claude hook evaluation can deny an eligible noisy command and provide wrapping guidance, but it never grants command authority.

Pull-request and CI artifacts contain untrusted code. Checksums protect transfer integrity; they do not make a PR build trusted. Stable installation links and agent setup should never be replaced automatically by a test artifact.
