#!/bin/sh
set -eu

REPOSITORY="${TAPAS_REPOSITORY:-nkootstra/tapas}"
INSTALL_DIR="${TAPAS_INSTALL_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
PR_NUMBER=""
VERSION=""
HEAD_SHA=""
CLEAN_PR=0
DRY_RUN=0

usage() {
    echo "usage: install.sh [--pr NUMBER] [--version TAG] [--clean-pr [--dry-run]]" >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --pr)
            [ "$#" -ge 2 ] || usage
            PR_NUMBER="$2"
            shift 2
            ;;
        --version)
            [ "$#" -ge 2 ] || usage
            VERSION="$2"
            shift 2
            ;;
        --clean-pr)
            CLEAN_PR=1
            shift
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --help|-h)
            usage
            ;;
        *)
            usage
            ;;
    esac
done

case "$PR_NUMBER" in
    ""|*[!0-9]*)
        [ -z "$PR_NUMBER" ] || { echo "PR number must be numeric" >&2; exit 2; }
        ;;
esac

if [ "$CLEAN_PR" -eq 1 ]; then
    [ -z "$PR_NUMBER" ] && [ -z "$VERSION" ] || usage
    found=0
    for candidate in "$INSTALL_DIR"/tapas-pr-*; do
        [ -f "$candidate" ] || continue
        found=1
        if [ "$DRY_RUN" -eq 1 ]; then
            echo "would remove $candidate"
        else
            rm -f -- "$candidate"
            echo "removed $candidate"
        fi
    done
    if [ "$found" -eq 0 ]; then
        echo "no local PR builds found in $INSTALL_DIR"
    fi
    exit 0
fi

[ "$DRY_RUN" -eq 0 ] || usage

case "$(uname -s):$(uname -m)" in
    Darwin:arm64|Darwin:aarch64) TARGET="aarch64-apple-darwin" ;;
    Linux:x86_64|Linux:amd64) TARGET="x86_64-unknown-linux-musl" ;;
    Linux:arm64|Linux:aarch64) TARGET="aarch64-unknown-linux-musl" ;;
    *)
        echo "unsupported platform: $(uname -s) $(uname -m)" >&2
        exit 1
        ;;
esac

command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
command -v tar >/dev/null 2>&1 || { echo "tar is required" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 1; }

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tapas-install.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

API="https://api.github.com/repos/$REPOSITORY"
curl -fsSL "$API" >/dev/null

if [ -n "$PR_NUMBER" ]; then
    PR_JSON="$TMP_DIR/pr.json"
    curl -fsSL "$API/pulls/$PR_NUMBER" -o "$PR_JSON"
    HEAD_SHA="$(python3 - "$PR_JSON" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    print(json.load(handle)["head"]["sha"])
PY
)"
    RELEASE_TAG="pr-${PR_NUMBER}-${HEAD_SHA}"
    RELEASE_JSON="$TMP_DIR/release.json"
    curl -fsSL "$API/releases/tags/$RELEASE_TAG" -o "$RELEASE_JSON" || {
        echo "no published PR build for #$PR_NUMBER at $HEAD_SHA yet" >&2
        exit 1
    }
    VERSION_LABEL="PR #$PR_NUMBER ($HEAD_SHA)"
    BINARY_NAME="tapas-pr-$(printf '%s' "$HEAD_SHA" | cut -c1-8)"
else
    if [ -n "$VERSION" ]; then
        RELEASE_TAG="$VERSION"
        case "$RELEASE_TAG" in v*) ;; *) RELEASE_TAG="v$RELEASE_TAG" ;; esac
        RELEASE_JSON="$TMP_DIR/release.json"
        curl -fsSL "$API/releases/tags/$RELEASE_TAG" -o "$RELEASE_JSON"
    else
        RELEASE_JSON="$TMP_DIR/releases.json"
        curl -fsSL "$API/releases?per_page=100" -o "$RELEASE_JSON"
        RELEASE_TAG="$(python3 - "$RELEASE_JSON" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    releases = json.load(handle)
for release in releases:
    if not release.get("draft") and not release.get("prerelease"):
        print(release["tag_name"])
        break
else:
    raise SystemExit("no stable release found")
PY
)"
        curl -fsSL "$API/releases/tags/$RELEASE_TAG" -o "$TMP_DIR/release.json"
    fi
    VERSION_LABEL="$RELEASE_TAG"
    BINARY_NAME="tapas"
fi

ASSET="tapas-${TARGET}.tar.gz"
ASSET_URL="https://github.com/$REPOSITORY/releases/download/$RELEASE_TAG/$ASSET"
curl -fsSL "$ASSET_URL" -o "$TMP_DIR/$ASSET"
curl -fsSL "https://github.com/$REPOSITORY/releases/download/$RELEASE_TAG/SHA256SUMS" -o "$TMP_DIR/SHA256SUMS"

EXPECTED="$(awk -v name="$ASSET" '$2 == name { print $1 }' "$TMP_DIR/SHA256SUMS")"
[ -n "$EXPECTED" ] || { echo "release checksum missing for $ASSET" >&2; exit 1; }
ACTUAL="$(sha256 "$TMP_DIR/$ASSET")"
[ "$EXPECTED" = "$ACTUAL" ] || { echo "release checksum mismatch" >&2; exit 1; }

mkdir -p "$TMP_DIR/unpacked" "$INSTALL_DIR"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR/unpacked"

METADATA="$TMP_DIR/unpacked/BUILD-METADATA.json"
[ -f "$METADATA" ] || { echo "release metadata missing" >&2; exit 1; }
python3 - "$METADATA" "$TARGET" "$PR_NUMBER" "$HEAD_SHA" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    metadata = json.load(handle)
if metadata.get("target") != sys.argv[2]:
    raise SystemExit("release target metadata mismatch")
if sys.argv[3] and metadata.get("source_sha") != sys.argv[4]:
    raise SystemExit("PR build source SHA mismatch")
PY

BUILD_LABEL="$(python3 - "$METADATA" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    metadata = json.load(handle)
print(metadata.get("version_label", metadata["version"]))
PY
)"

cp "$TMP_DIR/unpacked/tapas" "$INSTALL_DIR/$BINARY_NAME"
chmod 755 "$INSTALL_DIR/$BINARY_NAME"
echo "installed $VERSION_LABEL as $INSTALL_DIR/$BINARY_NAME"
echo "version: $BUILD_LABEL"
echo "run: $BINARY_NAME --version"
