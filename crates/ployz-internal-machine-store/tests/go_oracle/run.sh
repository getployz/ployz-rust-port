#!/usr/bin/env bash
set -euo pipefail

crate_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
repo_dir="$(cd "$crate_dir/../.." && pwd)"
oracle_dir="$repo_dir/upstream/uncloud"
overlay_target="$oracle_dir/internal/machine/store/store_json_overlay_test.go"
overlay_file="$(mktemp)"
trap 'rm -f "$overlay_file"' EXIT

python3 - "$overlay_file" "$overlay_target" "$crate_dir/tests/go_oracle/store_json_test.go" <<'PY'
import json
import pathlib
import sys

overlay_file, target, source = sys.argv[1:]
pathlib.Path(overlay_file).write_text(json.dumps({"Replace": {target: source}}))
PY

cd "$oracle_dir"
mise exec -- go test -overlay "$overlay_file" ./internal/machine/store
