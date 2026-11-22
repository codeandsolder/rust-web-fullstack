#!/usr/bin/env bash
set -euo pipefail
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
git -C "$fixture" init -q
git -C "$fixture" config user.email fixture@example.invalid
git -C "$fixture" config user.name fixture
printf 'services:\n  postgres:\n    image: postgres:18-alpine\n' > "$fixture/docker-compose.yml"
printf '[workspace]\n' > "$fixture/Cargo.toml"
printf '[toolchain]\nchannel = "nightly-2026-11-19"\n' > "$fixture/rust-toolchain.toml"
git -C "$fixture" add .
git -C "$fixture" -c commit.gpgsign=false commit -qm base
base="$(git -C "$fixture" rev-parse HEAD)"
assert_scope() {
  : > "$fixture/output"
  GITHUB_OUTPUT="$fixture/output" bash .github/scripts/classify-ci.sh "$1" "$fixture"
  diff -u <(printf 'postgres-only=%s\ncompose=%s\ncompiler-changed=%s\n' "$2" "$3" "$4") "$fixture/output"
}
sed -i s/18-alpine/19-alpine/ "$fixture/docker-compose.yml"
git -C "$fixture" -c commit.gpgsign=false commit -qam 'image bump'
assert_scope "$base" true true false
printf '# changed\n' >> "$fixture/Cargo.toml"
git -C "$fixture" -c commit.gpgsign=false commit -qam mixed
assert_scope "$base" false true false
git -C "$fixture" reset --hard -q "$base"
printf '    ports: ["5432:5432"]\n' >> "$fixture/docker-compose.yml"
git -C "$fixture" -c commit.gpgsign=false commit -qam compose
assert_scope "$base" false true false
git -C "$fixture" reset --hard -q "$base"
printf '# change\n' >> "$fixture/Cargo.toml"
git -C "$fixture" -c commit.gpgsign=false commit -qam cargo
assert_scope "$base" false false false
assert_scope "" false true false
# A native compiler bump is full Rust validation, even when bundled with PostgreSQL.
git -C "$fixture" reset --hard -q "$base"
sed -i s/2026-11-19/2026-11-20/ "$fixture/rust-toolchain.toml"
git -C "$fixture" -c commit.gpgsign=false commit -qam compiler
assert_scope "$base" false false true
sed -i s/18-alpine/19-alpine/ "$fixture/docker-compose.yml"
git -C "$fixture" -c commit.gpgsign=false commit -qam compiler-and-image
assert_scope "$base" false true true
if GITHUB_OUTPUT="$fixture/output" bash .github/scripts/classify-ci.sh invalid-base "$fixture" >/dev/null 2>&1; then
  echo 'Invalid base unexpectedly accepted' >&2
  exit 1
fi
