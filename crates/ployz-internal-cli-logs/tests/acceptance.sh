#!/bin/sh
set -eu

repo=$(git rev-parse --show-toplevel)
owned="$repo/crates/ployz-internal-cli-logs"
jobs=${CARGO_BUILD_JOBS:-3}
probe=$(mktemp -d /tmp/ployz-internal-cli-logs-acceptance.XXXXXX)
trap 'rm -rf "$probe"' EXIT HUP INT TERM

test -z "$(git -C "$repo" status --porcelain -- upstream/uncloud)"

mkdir -p "$probe/workspace/crates"
for crate in \
  ployz-internal-cli-logs \
  ployz-internal-cli-tui \
  ployz-pkg-api \
  ployz-internal-machine-api-pb
do
  cp -a "$repo/crates/$crate" "$probe/workspace/crates/$crate"
done
printf '%s\n' '[workspace]' 'resolver = "2"' \
  'members = ["crates/ployz-internal-cli-logs"]' > "$probe/workspace/Cargo.toml"

export CARGO_BUILD_JOBS="$jobs"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/tmp/ployz-internal-cli-logs-target}
cargo +1.96.0 fmt --manifest-path "$probe/workspace/Cargo.toml" --all --check
cargo +1.96.0 check --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-cli-logs --all-targets
cargo +1.96.0 test --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-cli-logs --all-targets
cargo +1.96.0 clippy --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-cli-logs --all-targets --all-features -- -D warnings

for target in \
  x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
  x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-gnu
do
  cargo +1.96.0 check --manifest-path "$probe/workspace/Cargo.toml" \
    -p ployz-internal-cli-logs --lib --target "$target"
done

mkdir -p "$probe/go"
cp -a "$repo/upstream/uncloud/." "$probe/go/"
cp "$owned/tests/go_oracle/logs_oracle_test.go" \
  "$probe/go/internal/cli/logs/ployz_oracle_test.go"
(
  cd "$probe/go"
  GOTOOLCHAIN=local /opt/go1.26.1/bin/go test -count=1 ./internal/cli/logs
)

for zone in UTC America/New_York
do
  zone_label=$(printf '%s' "$zone" | tr / _)
  rust_fixture="$probe/rust-$zone_label.tsv"
  TZ="$zone" cargo +1.96.0 run --quiet \
    --manifest-path "$probe/workspace/Cargo.toml" \
    -p ployz-internal-cli-logs --example oracle_probe > "$rust_fixture"
  (
    cd "$probe/go"
    TZ="$zone" GOTOOLCHAIN=local PLOYZ_RUST_LOG_FIXTURES_IN="$rust_fixture" \
      /opt/go1.26.1/bin/go test -count=1 ./internal/cli/logs \
      -run '^TestFormatterFixturesMatchRust$'
  )
done
