#!/bin/sh
set -eu

SCRIPT_URL="https://github.com/nkootstra/tapas/raw/refs/heads/main/install.sh"
command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
case "${1:-}" in
    --help|-h)
        echo "usage: install-pr.sh PR_NUMBER" >&2
        exit 0
        ;;
esac
installer=$(mktemp)
trap 'rm -f "$installer"' 0
trap 'exit 1' HUP INT TERM
curl -fsSL "$SCRIPT_URL" > "$installer"
sh "$installer" --pr "$@"
