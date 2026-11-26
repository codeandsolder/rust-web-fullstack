#!/usr/bin/env bash
# Only an exact PostgreSQL image-reference substitution may skip Rust work.
set -euo pipefail
base=${1:-}
repo=${2:-.}
postgres_only=false
compose=true
compiler_changed=false
browser_toolchain_changed=false
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
    case "$path" in
      *.Dockerfile|scripts/test-e2e.sh|scripts/wasm-bindgen-version.sh) browser_toolchain_changed=true ;;
      Cargo.lock)
        # Compare schema pins; unrelated Rust lockfile changes stay routine.
        old_bindgen=$(git -C "$repo" show "$base:Cargo.lock" | sh scripts/wasm-bindgen-version.sh /dev/stdin) || old_bindgen=
        new_bindgen=$(sh scripts/wasm-bindgen-version.sh "$repo/Cargo.lock") || new_bindgen=
        if [[ -z "$old_bindgen" || -z "$new_bindgen" || "$old_bindgen" != "$new_bindgen" ]]; then
          browser_toolchain_changed=true
        fi
        ;;
    esac
  done
  image='^    image: postgres:[^[:space:]]+$'
  if [[ "${#changed[@]}" == 1 && "${changed[0]}" == docker-compose.yml ]] &&
     [[ "$(git -C "$repo" show "$base:docker-compose.yml" | sed -En "/$image/p" | wc -l)" == 1 ]] &&
     [[ "$(git -C "$repo" show HEAD:docker-compose.yml | sed -En "/$image/p" | wc -l)" == 1 ]] &&
     cmp -s <(git -C "$repo" show "$base:docker-compose.yml" | sed -E "s/$image/    image: postgres:CI_IMAGE/")             <(git -C "$repo" show HEAD:docker-compose.yml | sed -E "s/$image/    image: postgres:CI_IMAGE/"); then
    postgres_only=true
  fi
fi
printf 'postgres-only=%s\ncompose=%s\ncompiler-changed=%s\nbrowser-toolchain-changed=%s\n' \
  "$postgres_only" "$compose" "$compiler_changed" "$browser_toolchain_changed" >> "$GITHUB_OUTPUT"
