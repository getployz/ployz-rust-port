# Frozen Uncloud behavioral oracle

## Provenance

| Field | Value |
| --- | --- |
| Official repository | `https://github.com/psviderski/uncloud.git` |
| Upstream branch at retrieval | `main` |
| Commit | `b7e224a1eff98813b1d1a32034d977be24be994e` |
| Commit subject | `feat(compose): support stdin_open and tty (#419)` |
| Upstream commit time | `2026-07-30T21:50:54+10:00` |
| Retrieval time | `2026-08-10T12:58:12Z` |
| Git tree | `a1959e967bbde8577ed4a19d367e8ee4b1ecf2bd` |
| Checked-in location | `upstream/uncloud/` |

The commit was the value of both upstream `HEAD` and `refs/heads/main` when
retrieved. The directory is a direct Git-tree import: file contents, executable
bits, and the `CLAUDE.md -> AGENTS.md` symlink are unchanged. It is the sole
behavioral oracle for the port. Do not edit it, rename symbols within it, or
repair failures in it; make port behavior match it. The future product rename
from Uncloud to Ployz applies only to the port.

After checking out a commit containing this baseline, verify the import with:

```sh
test "$(git rev-parse HEAD:upstream/uncloud)" = \
  a1959e967bbde8577ed4a19d367e8ee4b1ecf2bd
```

## Reproduced environment

The baseline was exercised on 64-bit Linux (`linux/amd64`, kernel 6.8) using:

- mise `2026.3.7`, matching upstream CI;
- Go `1.26.1`, as pinned by `mise.toml`;
- Docker Engine and CLI `29.1.3` with BuildKit;
- CGO enabled and network access to Go module servers, Docker Hub, GHCR, and
  the Corrosion release image.

The environment can be recreated from the repository root with:

```sh
cd upstream/uncloud
mise trust
mise install
go version
docker version
```

The daemon supports Linux only. Upstream documents cross-compilation for
non-Linux development hosts. The end-to-end tests also require a running Docker
daemon that permits privileged UCinD containers, free loopback TCP ports, and
external image pulls. Tests that exercise real remote machines additionally
need SSH-accessible hosts and systemd, WireGuard, and Docker on those hosts;
those manual scenarios are not part of `make test`.

## Build and test baseline

The untouched detached upstream checkout was exercised on 2026-08-10 with:

```sh
go build -o /tmp/uncloud-build-uc ./cmd/uc
go build -o /tmp/uncloud-build-uncloudd ./cmd/uncloudd
mise run ucind:image
make test
```

Both binaries built successfully. The UCinD image also built successfully.
Package and unit tests passed. The full `make test` run reached the Docker-based
end-to-end suite and reported one failure:

```text
TestDeployment/container_crashes_on_startup_without_healthcheck
service_test.go:1484: An error is expected but got nil.
```

The failing test passed when immediately rerun alone with the race detector:

```sh
go test -shuffle=on -race -count=1 -v \
  -run '^TestDeployment/container_crashes_on_startup_without_healthcheck$' \
  ./test/e2e
```

This is recorded as observed timing-sensitive upstream behavior, not repaired.
The complete run otherwise passed all reported tests and left no
`ucind.managed` containers behind.
