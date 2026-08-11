#!/bin/sh
set -eu

repo=$(git rev-parse --show-toplevel)
oracle="$repo/upstream/uncloud"
test -z "$(git -C "$repo" status --porcelain -- upstream/uncloud)"

probe=$(mktemp -d /tmp/ployz-machine-api-pb-oracle.XXXXXX)
trap 'rm -rf "$probe"' EXIT HUP INT TERM
cp -a "$oracle/." "$probe/"
cp "$repo/crates/ployz-internal-machine-api-pb/tests/go_oracle/known_fields_test.go" \
  "$probe/internal/machine/api/pb/known_fields_test.go"

cd "$probe"
GOTOOLCHAIN=local /opt/go1.26.1/bin/go test -count=1 -v ./internal/machine/api/pb
