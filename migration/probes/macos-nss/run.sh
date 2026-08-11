#!/usr/bin/env bash
set -euo pipefail

fail() {
  printf 'macOS NSS probe: %s\n' "$*" >&2
  exit 1
}

if [[ $# -ne 2 ]]; then
  fail "usage: run.sh OUTPUT_DIRECTORY EXPECTED_MACHINE"
fi

output_directory="$1"
expected_machine="$2"
probe_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository="$(git -C "${probe_directory}" rev-parse --show-toplevel)"

case "${output_directory}" in
  "${repository}"|"${repository}"/*)
    fail "output directory must be outside the repository"
    ;;
esac

case "${expected_machine}" in
  x86_64)
    expected_go_arch="amd64"
    expected_rust_host="x86_64-apple-darwin"
    ;;
  arm64)
    expected_go_arch="arm64"
    expected_rust_host="aarch64-apple-darwin"
    ;;
  *)
    fail "unsupported expected machine ${expected_machine}"
    ;;
esac

mkdir -p "${output_directory}"

[[ "$(uname -s)" == "Darwin" ]] ||
  fail "expected Darwin, got $(uname -s)"

actual_machine="$(uname -m)"
[[ "${actual_machine}" == "${expected_machine}" ]] ||
  fail "runner label produced ${actual_machine}, expected ${expected_machine}"

export CGO_ENABLED=1
export GOTOOLCHAIN=local
export CARGO_TERM_COLOR=never
export CARGO_TARGET_DIR="${RUNNER_TEMP:-/tmp}/ployz-macos-nss-target-${expected_machine}"
export GOCACHE="${RUNNER_TEMP:-/tmp}/ployz-macos-nss-gocache-${expected_machine}"

expected_go_version="go version go1.26.1 darwin/${expected_go_arch}"
actual_go_version="$(go version)"
[[ "${actual_go_version}" == "${expected_go_version}" ]] ||
  fail "expected ${expected_go_version}, got ${actual_go_version}"

[[ "$(go env GOOS)" == "darwin" ]] ||
  fail "Go toolchain is not targeting Darwin"
[[ "$(go env GOARCH)" == "${expected_go_arch}" ]] ||
  fail "Go toolchain architecture does not match the runner"
[[ "$(go env CGO_ENABLED)" == "1" ]] ||
  fail "Go native os/user backend requires CGO_ENABLED=1"

rust_version="$(rustc +1.96.0 -Vv)"
grep -Fxq "release: 1.96.0" <<<"${rust_version}" ||
  fail "Rust release is not exactly 1.96.0"
grep -Fxq "host: ${expected_rust_host}" <<<"${rust_version}" ||
  fail "Rust host does not match the native runner"

source_status="$(
  git -C "${repository}" status --porcelain -- \
    upstream/uncloud \
    migration/probes/macos-nss
)"
[[ -z "${source_status}" ]] ||
  fail "oracle or probe source was dirty before verification"

{
  printf 'schema=ployz-macos-nss-metadata-v1\n'
  printf 'runner_machine=%s\n' "${actual_machine}"
  printf 'expected_go_arch=%s\n' "${expected_go_arch}"
  printf 'expected_rust_host=%s\n' "${expected_rust_host}"
  uname -a
  sw_vers
  printf '%s\n' "${actual_go_version}"
  go env GOOS GOARCH CGO_ENABLED
  printf '%s\n' "${rust_version}"
  cargo +1.96.0 -V
} >"${output_directory}/versions.txt"

current_name="$(id -un)"
current_uid="$(id -u)"

passwd_contains() {
  local name="$1"
  awk -F: -v expected="${name}" '
    $1 == expected { found = 1 }
    END { exit(found ? 0 : 1) }
  ' /etc/passwd
}

if passwd_contains "${current_name}"; then
  fail "current account ${current_name} exists in /etc/passwd; Open Directory provenance is unproven"
fi

dscl . -read "/Users/${current_name}" \
  RecordName UniqueID PrimaryGroupID NFSHomeDirectory \
  >"${output_directory}/current.dscl.txt"

dscacheutil -q user -a name "${current_name}" \
  >"${output_directory}/current.dscacheutil.txt"
grep -q '^name:' "${output_directory}/current.dscacheutil.txt" ||
  fail "dscacheutil did not resolve the current Open Directory account"

dscl . -list /Users UniqueID |
  LC_ALL=C sort >"${output_directory}/dscl-users.txt"

directory_name=""
directory_passwd_present=""
candidate_dscl="${output_directory}/candidate.dscl.tmp"
candidate_cache="${output_directory}/candidate.dscacheutil.tmp"

while IFS= read -r candidate; do
  [[ -n "${candidate}" ]] || continue
  [[ "${candidate}" != "${current_name}" ]] || continue

  if ! dscl . -read "/Users/${candidate}" \
    RecordName UniqueID PrimaryGroupID NFSHomeDirectory \
    >"${candidate_dscl}" 2>&1
  then
    continue
  fi

  candidate_uid="$(
    awk '$1 == "UniqueID:" { print $2; exit }' "${candidate_dscl}"
  )"
  case "${candidate_uid}" in
    ""|*[!0-9]*)
      continue
      ;;
  esac
  [[ "${candidate_uid}" != "${current_uid}" ]] || continue

  if ! dscacheutil -q user -a name "${candidate}" \
    >"${candidate_cache}" 2>&1
  then
    continue
  fi
  grep -q '^name:' "${candidate_cache}" || continue

  directory_name="${candidate}"
  if passwd_contains "${candidate}"; then
    directory_passwd_present="true"
  else
    directory_passwd_present="false"
  fi
  break
done < <(awk '{ print $1 }' "${output_directory}/dscl-users.txt")

[[ -n "${directory_name}" ]] ||
  fail "no second Directory Service account was available"

mv "${candidate_dscl}" "${output_directory}/directory.dscl.txt"
mv "${candidate_cache}" "${output_directory}/directory.dscacheutil.txt"

{
  printf 'schema=ployz-macos-nss-directory-source-v2\n'
  printf 'current_name=%s\n' "${current_name}"
  printf 'current_uid=%s\n' "${current_uid}"
  printf 'lookup_name=%s\n' "${directory_name}"
  printf 'current_absent_from_passwd=true\n'
  printf 'lookup_present_in_passwd=%s\n' "${directory_passwd_present}"
  printf 'current_dscl_resolved=true\n'
  printf 'lookup_dscl_resolved=true\n'
  printf 'current_dscacheutil_resolved=true\n'
  printf 'lookup_dscacheutil_resolved=true\n'
} >"${output_directory}/directory-source.txt"

CGO_ENABLED=1 GOTOOLCHAIN=local \
  go run "${probe_directory}/go/main.go" "${directory_name}" \
  >"${output_directory}/go.tsv"

cargo +1.96.0 fmt \
  --manifest-path "${probe_directory}/Cargo.toml" \
  -- --check

cargo +1.96.0 check \
  --manifest-path "${probe_directory}/Cargo.toml" \
  --locked \
  --all-targets

cargo +1.96.0 clippy \
  --manifest-path "${probe_directory}/Cargo.toml" \
  --locked \
  --all-targets \
  -- -D warnings

cargo +1.96.0 run \
  --manifest-path "${probe_directory}/Cargo.toml" \
  --locked \
  --quiet \
  -- "${directory_name}" \
  >"${output_directory}/rust.tsv"

if ! diff -u \
  "${output_directory}/go.tsv" \
  "${output_directory}/rust.tsv" \
  >"${output_directory}/differential.diff"
then
  cat "${output_directory}/differential.diff" >&2
  fail "Go and Rust native account results differ"
fi

source_status="$(
  git -C "${repository}" status --porcelain -- \
    upstream/uncloud \
    migration/probes/macos-nss
)"
[[ -z "${source_status}" ]] ||
  fail "verification modified oracle or probe source"

{
  printf 'schema=ployz-macos-nss-result-v1\n'
  printf 'status=pass\n'
  printf 'machine=%s\n' "${actual_machine}"
  printf 'go=1.26.1\n'
  printf 'rust=1.96.0\n'
  printf 'libc=0.2.189\n'
  printf 'records_compared=5\n'
} >"${output_directory}/result.txt"
