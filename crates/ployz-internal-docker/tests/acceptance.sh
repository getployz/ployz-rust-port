#!/bin/sh
set -eu

repo=$(git rev-parse --show-toplevel)
crate="$repo/crates/ployz-internal-docker"
jobs=${CARGO_BUILD_JOBS:-3}
target=${CARGO_TARGET_DIR:-/tmp/ployz-internal-docker-target}

test -z "$(git -C "$repo" status --porcelain -- upstream/uncloud)"

probe=/tmp/ployz-internal-docker-acceptance-workspace
rm -rf "$probe/workspace/crate"
mkdir -p "$probe/workspace"
cp -a "$crate" "$probe/workspace/crate"
printf '%s\n' '[workspace]' 'resolver = "2"' 'members = ["crate"]' \
  > "$probe/workspace/Cargo.toml"

tls="$probe/tls"
rm -rf "$tls"
mkdir -p "$tls"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj /CN=Ployz-Test-CA -keyout "$tls/ca.key" -out "$tls/ca.pem" \
  >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj /CN=127.0.0.1 \
  -addext subjectAltName=IP:127.0.0.1 \
  -addext extendedKeyUsage=serverAuth \
  -keyout "$tls/server.key" -out "$tls/server.csr" >/dev/null 2>&1
openssl x509 -req -days 1 -in "$tls/server.csr" -CA "$tls/ca.pem" \
  -CAkey "$tls/ca.key" -CAcreateserial -copy_extensions copy \
  -out "$tls/server.pem" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj /CN=ployz-client \
  -addext extendedKeyUsage=clientAuth \
  -keyout "$tls/key.pem" -out "$tls/client.csr" >/dev/null 2>&1
openssl x509 -req -days 1 -in "$tls/client.csr" -CA "$tls/ca.pem" \
  -CAkey "$tls/ca.key" -CAcreateserial -copy_extensions copy \
  -out "$tls/cert.pem" >/dev/null 2>&1

tls_port=$((24000 + $$ % 20000))
tls_pid=
cleanup() {
  if test -n "$tls_pid"; then
    kill "$tls_pid" 2>/dev/null || true
    wait "$tls_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM
/opt/go1.26.1/bin/go build -o "$tls/server" "$crate/tests/tls_server/main.go"
"$tls/server" "127.0.0.1:$tls_port" "$tls" >"$tls/server.log" 2>&1 &
tls_pid=$!

CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR="$target" cargo +1.96.0 fmt \
  --manifest-path "$probe/workspace/Cargo.toml" --all --check
CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR="$target" cargo +1.96.0 check \
  --manifest-path "$probe/workspace/Cargo.toml" --workspace --all-targets
PLOYZ_RUST_PROGRESS_OUT="$probe/rust-progress.tsv" \
  CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR="$target" cargo +1.96.0 test \
  --manifest-path "$probe/workspace/Cargo.toml" --workspace --all-targets

DOCKER_HOST="tcp://127.0.0.1:$tls_port" DOCKER_TLS_VERIFY=1 \
  DOCKER_CERT_PATH="$tls" DOCKER_API_VERSION=1.53 \
  CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR="$target" cargo +1.96.0 run \
  --manifest-path "$probe/workspace/Cargo.toml" --package ployz-internal-docker \
  --example tls_probe

mkdir -p "$probe/go"
cp -a "$repo/upstream/uncloud/." "$probe/go/"
cp "$crate/tests/go_oracle/docker_oracle_test.go" \
  "$probe/go/internal/docker/docker_oracle_test.go"
mkdir -p "$probe/go/internal/docker/testdata"
cp "$crate/tests/fixtures/progress.stream.json" \
  "$probe/go/internal/docker/testdata/progress.stream.json"
(
  cd "$probe/go"
  GOTOOLCHAIN=local PLOYZ_RUST_PROGRESS_OUT="$probe/rust-progress.tsv" \
    /opt/go1.26.1/bin/go test -count=1 ./internal/docker
)

CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR="$target" cargo +1.96.0 clippy \
  --manifest-path "$probe/workspace/Cargo.toml" --workspace --all-targets --all-features -- -D warnings

for rust_target in aarch64-unknown-linux-gnu x86_64-pc-windows-gnu; do
  CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR="$target" cargo +1.96.0 check \
    --manifest-path "$probe/workspace/Cargo.toml" --workspace --all-targets \
    --target "$rust_target"
done
