#!/bin/sh
set -eu

repo=$(git rev-parse --show-toplevel)
crate="$repo/crates/ployz-internal-machine-api-pb"
oracle="$repo/upstream/uncloud/internal/machine/api"

test -z "$(git -C "$repo" status --porcelain -- upstream/uncloud)"
for proto in caddy cluster common docker machine; do
  cmp "$oracle/pb/$proto.proto" \
    "$crate/proto/internal/machine/api/pb/$proto.proto"
done
cmp "$oracle/vendor/google/rpc/status.proto" \
  "$crate/proto/google/rpc/status.proto"
