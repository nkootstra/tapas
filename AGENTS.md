# Repository Instructions

Pull request titles may use one of these release-intent prefixes:

- `major:` for breaking changes
- `minor:` for backward-compatible features
- `patch:` for backward-compatible fixes
- `skip:` for changes that should not trigger a release

An unprefixed title does not trigger a release; its change is included when a later
`major:`, `minor:`, or `patch:` pull request triggers one. `skip:` changes are omitted
from the changelog. When present, a prefix must be lowercase and followed by a colon and one space, then
a concise description, for example: `minor: add command coverage`.

Never add promotional, model-generated, or tool-generated attribution blocks to pull request descriptions, commit messages, or changelogs.
