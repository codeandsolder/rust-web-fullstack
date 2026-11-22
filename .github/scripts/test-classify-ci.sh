#!/usr/bin/env bash
set -euo pipefail
classifier="$(realpath "$(dirname "$0")/classify-ci.sh")"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
cd "$fixture"
git init -q
git config user.email fixture@example.invalid
git config user.name fixture
printf 'services:\n  postgres:\n    image: postgres:18-alpine\n' > docker-compose.yml
printf '[workspace]\n' > Cargo.toml
git add .
git -c commit.gpgsign=false commit -qm base
base="$(git rev-parse HEAD)"
assert_scope() {
  : > "$fixture/output"
  GITHUB_OUTPUT="$fixture/output" bash "$classifier" "$1"
  diff -u <(printf 'postgres-only=%s\ncompose=%s\n' "$2" "$3") "$fixture/output"
}
sed -i s/18-alpine/19-alpine/ docker-compose.yml
git -c commit.gpgsign=false commit -qam 'image bump'
assert_scope "$base" true true
printf '# changed\n' >> Cargo.toml
git -c commit.gpgsign=false commit -qam mixed
assert_scope "$base" false true
git reset --hard -q "$base"
printf '    ports: ["5432:5432"]\n' >> docker-compose.yml
git -c commit.gpgsign=false commit -qam compose
assert_scope "$base" false true
git reset --hard -q "$base"
printf '# change\n' >> Cargo.toml
git -c commit.gpgsign=false commit -qam cargo
assert_scope "$base" false false
assert_scope "" false true
if GITHUB_OUTPUT="$fixture/output" bash "$classifier" invalid-base >/dev/null 2>&1; then
  echo 'Invalid base unexpectedly accepted' >&2
  exit 1
fi
