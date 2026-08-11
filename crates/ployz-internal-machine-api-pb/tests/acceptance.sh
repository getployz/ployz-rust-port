#!/bin/sh
set -eu

repo=$(git rev-parse --show-toplevel)
crate="$repo/crates/ployz-internal-machine-api-pb"
oracle="$repo/upstream/uncloud"
jobs=${CARGO_BUILD_JOBS:-3}

test -z "$(git -C "$repo" status --porcelain -- upstream/uncloud)"
command -v protoc >/dev/null
"$crate/tests/verify_schema_snapshot.sh"

probe=$(mktemp -d /tmp/ployz-machine-api-pb-acceptance.XXXXXX)
trap 'rm -rf "$probe"' EXIT HUP INT TERM
mkdir -p "$probe/workspace"
cp -a "$crate" "$probe/workspace/crate"
printf '%s\n' '[workspace]' 'resolver = "2"' 'members = ["crate"]' \
  > "$probe/workspace/Cargo.toml"

CARGO_BUILD_JOBS="$jobs" PLOYZ_PROTO_VERIFY=1 \
  cargo +1.96.0 check --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-machine-api-pb --all-targets

CARGO_BUILD_JOBS="$jobs" PLOYZ_RUST_FIXTURES_OUT="$probe/rust.tsv" \
  cargo +1.96.0 test --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-machine-api-pb --test known_fields \
  go_known_field_fixtures_decode_and_round_trip_semantically

mkdir -p "$probe/go"
cp -a "$oracle/." "$probe/go/"
cp "$crate/tests/go_oracle/known_fields_test.go" \
  "$probe/go/internal/machine/api/pb/known_fields_test.go"
(
  cd "$probe/go"
  GOTOOLCHAIN=local \
    PLOYZ_RUST_FIXTURES_IN="$probe/rust.tsv" \
    PLOYZ_GO_FIXTURES_OUT="$probe/go.tsv" \
    /opt/go1.26.1/bin/go test -count=1 ./internal/machine/api/pb
)

CARGO_BUILD_JOBS="$jobs" PLOYZ_GO_FIXTURES_IN="$probe/go.tsv" \
  cargo +1.96.0 test --manifest-path "$probe/workspace/Cargo.toml" \
  -p ployz-internal-machine-api-pb --test known_fields \
  go_known_field_fixtures_decode_and_round_trip_semantically
