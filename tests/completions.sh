#!/usr/bin/env bash
set -euo pipefail

binary=${1:?usage: tests/completions.sh <tapas-binary>}
binary=$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")
completion_cwd=$(mktemp -d)
trap 'rmdir "$completion_cwd"' EXIT

normalize_candidates() {
  sed $'s/\t.*//' | sed '/^$/d' | sort
}

bash_candidates() {
  bash -c '
    source <("$1" --completions bash)
    read -r -a COMP_WORDS <<< "$2"
    COMP_CWORD=$((${#COMP_WORDS[@]} - 1))
    _tapas
    printf "%s\n" "${COMPREPLY[@]}"
  ' bash "$binary" "$1"
}

zsh_candidates() {
  zsh -c '
    compdef() { :; }
    typeset -ga captured
    compadd() {
      shift
      local candidate prefix="$words[$CURRENT]"
      for candidate in "$@"; do
        [[ "$candidate" == "$prefix"* ]] && captured+=("$candidate")
      done
    }
    source <("$1" --completions zsh)
    words=(${=2})
    CURRENT=${#words[@]}
    _tapas
    print -rl -- "${captured[@]}"
  ' zsh "$binary" "$1"
}

fish_candidates() {
  fish -c '
    cd "$argv[3]"
    "$argv[1]" --completions fish | source
    complete -C "$argv[2]"
  ' "$binary" "$1" "$completion_cwd"
}

assert_scenario() {
  local label=$1
  local command_line=$2
  local expected=$3
  local shell actual normalized_expected

  normalized_expected=$(printf '%s\n' "$expected" | normalize_candidates)
  for shell in bash zsh fish; do
    actual=$("${shell}_candidates" "$command_line" | normalize_candidates)
    if [[ "$actual" != "$normalized_expected" ]]; then
      printf '%s %s completion mismatch\nexpected:\n%s\nactual:\n%s\n' \
        "$shell" "$label" "$normalized_expected" "$actual" >&2
      return 1
    fi
  done
}

assert_scenario \
  'top level' \
  'tapas -' \
  $'-h\n--help\n--version\n--filters\n--raw\n--explain\n--rewrite\n--hook-eval\n--setup\n--unsetup\n--completions'
assert_scenario 'completion shells' 'tapas --completions b' 'bash'
assert_scenario 'hook targets' 'tapas --hook-eval c' $'claude\ncodex'
assert_scenario 'attached setup target' 'tapas --setup=o' '--setup=opencode'
assert_scenario 'attached unsetup target' 'tapas --unsetup=c' $'--unsetup=claude\n--unsetup=codex'
assert_scenario 'OpenCode setup' 'tapas --setup opencode --' $'--dry-run\n--force'
assert_scenario 'attached OpenCode setup' 'tapas --setup=opencode --' $'--dry-run\n--force'
assert_scenario 'setup after dry-run' 'tapas --setup opencode --dry-run --' '--force'
assert_scenario 'setup after force' 'tapas --setup opencode --force --' '--dry-run'
assert_scenario 'setup after all options' 'tapas --setup opencode --dry-run --force --' ''
assert_scenario 'Claude setup' 'tapas --setup claude --' '--dry-run'
assert_scenario 'unsetup' 'tapas --unsetup opencode --' '--dry-run'
assert_scenario 'unsetup after dry-run' 'tapas --unsetup opencode --dry-run --' ''
assert_scenario 'raw separator' 'tapas --raw -' '--'
assert_scenario 'wrapped command' 'tapas git --' ''
