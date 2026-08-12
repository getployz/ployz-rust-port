#!/bin/sh
set -eu

repo=$(git rev-parse --show-toplevel)
owned="$repo/crates/ployz-internal-machine-api-proxy"
protobuf="$repo/crates/ployz-internal-machine-api-pb"
oracle="$repo/upstream/uncloud"
jobs=${CARGO_BUILD_JOBS:-3}

test -z "$(git -C "$repo" status --porcelain -- upstream/uncloud)"
probe=$(mktemp -d /tmp/ployz-machine-api-proxy-acceptance.XXXXXX)
trap 'rm -rf "$probe"' EXIT HUP INT TERM
mkdir -p "$probe/workspace/crates"
cp -a "$owned" "$probe/workspace/crates/ployz-internal-machine-api-proxy"
cp -a "$protobuf" "$probe/workspace/crates/ployz-internal-machine-api-pb"
printf '%s\n' '[workspace]' 'resolver = "2"' \
  'members = ["crates/ployz-internal-machine-api-pb", "crates/ployz-internal-machine-api-proxy"]' \
  > "$probe/workspace/Cargo.toml"

export CARGO_BUILD_JOBS="$jobs"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/tmp/ployz-machine-api-proxy-target}
cargo +1.96.0 fmt --manifest-path "$probe/workspace/Cargo.toml" --all --check
cargo +1.96.0 check --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-machine-api-proxy --all-targets
cargo +1.96.0 test --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-machine-api-proxy --all-targets
cargo +1.96.0 clippy --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-machine-api-proxy --all-targets --all-features -- -D warnings
for target in \
  x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
  x86_64-apple-darwin aarch64-apple-darwin
do
  cargo +1.96.0 check --manifest-path "$probe/workspace/Cargo.toml" \
    -p ployz-internal-machine-api-proxy --lib --target "$target"
done

mkdir -p "$probe/go"
cp -a "$oracle/." "$probe/go/"
cp "$owned/tests/go_oracle/probe_test.go" \
  "$probe/go/internal/machine/api/proxy/ployz_probe_test.go"
(
  cd "$probe/go"
  GOTOOLCHAIN=local PLOYZ_GO_FIXTURES_OUT="$probe/go.tsv" \
    /opt/go1.26.1/bin/go test -count=1 ./internal/machine/api/proxy
)
PLOYZ_GO_FIXTURES_IN="$probe/go.tsv" \
  cargo +1.96.0 test --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-machine-api-proxy --lib \
  go_payload_fixtures_match_byte_for_byte
