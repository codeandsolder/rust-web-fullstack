#!/usr/bin/env bash
# Only an exact PostgreSQL image-reference substitution may skip Rust work.
set -euo pipefail
base=${1:-}
repo=${2:-.}
postgres_only=false
compose=true
compiler_changed=false
if [[ -n "$base" ]]; then
  git -C "$repo" cat-file -e "$base^{commit}"
  changes=$(mktemp)
  trap 'rm -f "$changes"' EXIT
  git -C "$repo" diff --no-renames --name-only -z "$base" HEAD > "$changes"
  mapfile -d '' -t changed < "$changes"
  compose=false
  for path in "${changed[@]}"; do
    if [[ "$path" == docker-compose.yml ]]; then compose=true; fi
    if [[ "$path" == rust-toolchain.toml ]]; then compiler_changed=true; fi
  done
  image='^    image: postgres:[^[:space:]]+$'
  if [[ "${#changed[@]}" == 1 && "${changed[0]}" == docker-compose.yml ]] &&
     [[ "$(git -C "$repo" show "$base:docker-compose.yml" | sed -En "/$image/p" | wc -l)" == 1 ]] &&
     [[ "$(git -C "$repo" show HEAD:docker-compose.yml | sed -En "/$image/p" | wc -l)" == 1 ]] &&
     cmp -s <(git -C "$repo" show "$base:docker-compose.yml" | sed -E "s/$image/    image: postgres:CI_IMAGE/")             <(git -C "$repo" show HEAD:docker-compose.yml | sed -E "s/$image/    image: postgres:CI_IMAGE/"); then
    postgres_only=true
  fi
fi
printf 'postgres-only=%s\ncompose=%s\ncompiler-changed=%s\n' \
  "$postgres_only" "$compose" "$compiler_changed" >> "$GITHUB_OUTPUT"
