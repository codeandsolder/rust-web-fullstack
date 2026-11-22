#!/usr/bin/env bash
# Only an exact PostgreSQL image-reference substitution may skip Rust work.
set -euo pipefail
base=${1:-}
postgres_only=false
compose=true
if [[ -n "$base" ]]; then
  git cat-file -e "$base^{commit}"
  changes=$(mktemp)
  trap 'rm -f "$changes"' EXIT
  git diff --name-only -z "$base" HEAD > "$changes"
  mapfile -d '' -t changed < "$changes"
  compose=false
  for path in "${changed[@]}"; do
    if [[ "$path" == docker-compose.yml ]]; then compose=true; fi
  done
  image='^    image: postgres:[^[:space:]]+$'
  if [[ "${#changed[@]}" == 1 && "${changed[0]}" == docker-compose.yml ]] &&
     [[ "$(git show "$base:docker-compose.yml" | rg -c "$image")" == 1 ]] &&
     [[ "$(rg -c "$image" docker-compose.yml)" == 1 ]] &&
     cmp -s <(git show "$base:docker-compose.yml" | sed -E "s/$image/    image: postgres:CI_IMAGE/")             <(sed -E "s/$image/    image: postgres:CI_IMAGE/" docker-compose.yml); then
    postgres_only=true
  fi
fi
printf 'postgres-only=%s\ncompose=%s\n' "$postgres_only" "$compose" >> "$GITHUB_OUTPUT"
