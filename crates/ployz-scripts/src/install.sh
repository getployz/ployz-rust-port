#!/usr/bin/env bash

set -euo pipefail

# Avoid locale warnings (e.g. on apt install) from SSH-forwarded LANG/LC_* on minimal hosts. C.UTF-8 is built into
# glibc and present on all supported distros, unlike locale-specific values like en_US.UTF-8 that may not be generated.
export LC_ALL="${LC_ALL:-C.UTF-8}"

# If set to 'true', only install the packages and dependencies, without running, reloading, or
# restarting services or systemd.
INSTALL_ONLY=${INSTALL_ONLY:-false}

INSTALL_BIN_DIR=${INSTALL_BIN_DIR:-/usr/local/bin}
INSTALL_SYSTEMD_DIR=${INSTALL_SYSTEMD_DIR:-/etc/systemd/system}

PLOYZ_GITHUB_URL="https://github.com/getployz/ployz"
PLOYZ_VERSION=${PLOYZ_VERSION:-latest}
# Remove the 'v' prefix from the version if it exists.
PLOYZ_VERSION=${PLOYZ_VERSION#v}
PLOYZ_USER="ployz"
# Add the specified Linux user to group $PLOYZ_USER to allow the user to run ployz commands without sudo.
PLOYZ_GROUP_ADD_USER=${PLOYZ_GROUP_ADD_USER:-}
PLOYZ_DATA_DIR=${PLOYZ_DATA_DIR:-/var/lib/ployz}

DOCKER_ALREADY_INSTALLED=false
CONTAINERD_IMAGE_STORE_ENABLED=false
DOCKER_DAEMON_CONFIG_FILE=${DOCKER_DAEMON_CONFIG_FILE:-/etc/docker/daemon.json}
# Docker daemon configuration optimised for Ployz.
DOCKER_DAEMON_CONFIG='{
  "features": {
    "containerd-snapshotter": true
  },
  "live-restore": true
}'

log() {
    echo -e "\033[1;32m$1\033[0m"
}

warning() {
    echo -e "\033[1;33m$1\033[0m"
}

error() {
    echo -e "\033[1;31mERROR: $1\033[0m" >&2
    exit 1
}

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

verify_system() {
  if [[ "$(uname -s)" != "Linux" ]]; then
      error "Ployz machine must be a Linux system. Your system ($(uname -s)) is not supported."
  fi

  local arch
  arch=$(uname -m)
  if [[ "$arch" != "x86_64" && "$arch" != "aarch64" ]]; then
      error "Ployz machine must have amd64 (x86_64) or arm64 (aarch64) architecture. \
Your system architecture ($arch) is not supported."
  fi

  if [[ ! -d /run/systemd/system && "${INSTALL_ONLY}" != "true" ]]; then
      error "Cannot find systemd to use as a service manager for the Ployz machine daemon. \
Ployz supports only systemd-based Linux systems for now."
  fi
}

# install_prerequisites ensures curl is available, installing it with the system package manager if needed.
install_prerequisites() {
    if command_exists curl; then
        return
    fi

    log "⏳ curl is required but not installed, installing it using the system package manager..."

    if command_exists apt-get; then
        apt-get update -qq >/dev/null
        DEBIAN_FRONTEND=noninteractive apt-get install -y -qq curl ca-certificates
    elif command_exists dnf; then
        dnf install -y curl ca-certificates
    elif command_exists yum; then
        yum install -y curl ca-certificates
    elif command_exists pacman; then
        pacman -Sy --noconfirm curl ca-certificates
    elif command_exists zypper; then
        zypper --non-interactive refresh >/dev/null
        zypper --non-interactive install curl ca-certificates
    else
        error "curl is required but not installed, and no supported package manager \
(apt, dnf, yum, pacman, zypper) was found to install it automatically. \
Please install curl manually and re-run the command."
    fi

    log "✓ curl installed."
}

install_docker() {
    if command_exists dockerd; then
        log "✓ Docker is already installed."
        DOCKER_ALREADY_INSTALLED=true

        if [[ "${INSTALL_ONLY}" == "true" ]]; then
            return
        fi

        docker version

        # Check if the installed Docker configured to use the containerd image store.
        local driver_status
        driver_status=$(docker info -f '{{ .DriverStatus }}' 2>/dev/null)
        if [[ "$driver_status" == *"io.containerd.snapshotter"* ]]; then
            CONTAINERD_IMAGE_STORE_ENABLED="true"
        fi

        return
    fi

    log "⏳ Installing Docker..."
    curl -fsSL https://get.docker.com | sh

    # Configure Docker daemon for new installation.
    # Create Docker daemon config directory if it doesn't exist.
    local docker_config_dir
    docker_config_dir=$(dirname "${DOCKER_DAEMON_CONFIG_FILE}")
    if [ ! -d "${docker_config_dir}" ]; then
        mkdir -p "${docker_config_dir}"
    fi

    log "⏳ Configuring Docker daemon (${DOCKER_DAEMON_CONFIG_FILE}) to optimise it for Ployz..."
    echo "${DOCKER_DAEMON_CONFIG}" > "${DOCKER_DAEMON_CONFIG_FILE}"

    if [[ "${INSTALL_ONLY}" != "true" ]]; then
        systemctl restart docker
    fi

    log "✓ Docker installed and configured successfully."
}

create_ployz_user_and_group() {
    if id "${PLOYZ_USER}" &> /dev/null; then
        log "✓ Linux user '${PLOYZ_USER}' already exists."
    else
        # In addition to creating the user, create a group with the same name as the user.
        if ! useradd --system --home-dir /nonexistent --shell /usr/sbin/nologin --user-group "${PLOYZ_USER}"; then
            error "Failed to create Linux user '${PLOYZ_USER}'."
        fi
        log "✓ Linux user and group '${PLOYZ_USER}' created."
    fi

    if [ -n "${PLOYZ_GROUP_ADD_USER}" ]; then
        if ! gpasswd --add "${PLOYZ_GROUP_ADD_USER}" "${PLOYZ_USER}" > /dev/null; then
            error "Failed to add user '${PLOYZ_GROUP_ADD_USER}' to group '${PLOYZ_USER}'."
        fi
        log "✓ Linux user '${PLOYZ_GROUP_ADD_USER}' added to group '${PLOYZ_USER}'."
    fi
}

install_ployz_binaries() {
    local arch
    local file_arch

    arch=$(uname -m)
    case $arch in
        x86_64)
            file_arch="amd64"
            ;;
        aarch64)
            file_arch="arm64"
            ;;
        *)
            error "Unsupported architecture: ${arch}"
            ;;
    esac

    local ployzd_install_path="${INSTALL_BIN_DIR}/ployzd"
    local installed_version=""
    if [ -f "${ployzd_install_path}" ]; then
        installed_version=$("${ployzd_install_path}" version -o '{{.Version}}' 2>/dev/null || true)
    fi

    # Decide whether to (re)install based on PLOYZ_VERSION and the already installed binary, and pick the
    # download URL for the same channel/tag.
    local ployzd_url uninstall_ref
    local ployzd_archive_name="ployzd_linux_${file_arch}.tar.gz"

    case "${PLOYZ_VERSION}" in
        nightly)
            log "⏳ Installing ployzd from the nightly channel (installed: ${installed_version:-none})..."
            ployzd_url="${PLOYZ_GITHUB_URL}/releases/download/nightly/${ployzd_archive_name}"
            # The 'nightly' tag is deleted and recreated on every nightly build, so pin uninstall.sh to main
            # to avoid racing.
            uninstall_ref="refs/heads/main"
            ;;
        latest)
            # Resolve the redirect of releases/latest to discover the concrete latest version.
            local latest_url latest_version
            latest_url=$(curl -sLI -o /dev/null -w '%{url_effective}' \
                "${PLOYZ_GITHUB_URL}/releases/latest" 2>/dev/null || true)
            latest_version="${latest_url##*/}"
            latest_version="${latest_version#v}"

            uninstall_ref="refs/tags/v${latest_version}"

            if [ -z "${latest_version}" ] || [ "${latest_version}" = "latest" ]; then
                warning "Could not resolve the version of the latest release on GitHub."
                # Fall back to the redirecting URL so curl follows whatever GitHub considers latest.
                ployzd_url="${PLOYZ_GITHUB_URL}/releases/latest/download/${ployzd_archive_name}"
                uninstall_ref="refs/heads/main"
            elif [ -z "${installed_version}" ]; then
                log "⏳ Installing ployzd ${latest_version} (latest stable)..."
                ployzd_url="${PLOYZ_GITHUB_URL}/releases/download/v${latest_version}/${ployzd_archive_name}"
            elif [ "${installed_version}" = "${latest_version}" ]; then
                log "✓ ployzd ${installed_version} is already the latest stable version."
                return 0
            else
                # Substitute '-' with '~' so sort -V treats pre-release versions per SemVer (Debian-style rules):
                # 0.20.0~nightly-abc < 0.20.0 < 0.21.0~nightly-def.
                # latest_version is always a clean stable tag from releases/latest, so the substitution is one-sided.
                local newest
                newest=$(printf '%s\n%s\n' "${installed_version//-/\~}" "${latest_version}" | sort -V | tail -n1)
                if [ "${newest}" = "${latest_version}" ]; then
                    log "⏳ Upgrading ployzd ${installed_version} → ${latest_version}..."
                    ployzd_url="${PLOYZ_GITHUB_URL}/releases/download/v${latest_version}/${ployzd_archive_name}"
                else
                    log "✓ ployzd ${installed_version} is newer than the latest stable ${latest_version}, keeping it."
                    return 0
                fi
            fi
            ;;
        *)
            # Explicit version. Install if it differs from the installed one (covers upgrade and downgrade).
            if [ "${installed_version}" = "${PLOYZ_VERSION}" ]; then
                log "✓ ployzd ${installed_version} matches the requested version, keeping it."
                return 0
            fi
            log "⏳ Installing ployzd ${PLOYZ_VERSION} (replacing ${installed_version:-none})..."
            ployzd_url="${PLOYZ_GITHUB_URL}/releases/download/v${PLOYZ_VERSION}/${ployzd_archive_name}"
            uninstall_ref="refs/tags/v${PLOYZ_VERSION}"
            ;;
    esac
    local uninstall_url="https://raw.githubusercontent.com/getployz/ployz/${uninstall_ref}/scripts/uninstall.sh"

    # Create a temporary directory for downloads.
    local tmp_dir
    tmp_dir=$(mktemp -d)
    # Ensure the temporary directory is deleted on script exit.
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp_dir'" EXIT

    local ployzd_download_path="${tmp_dir}/ployzd.tar.gz"
    local uninstall_download_path="${tmp_dir}/uninstall.sh"

    log "⏳ Downloading ployzd binary: ${ployzd_url}"
    if ! curl -fsSL -o "${ployzd_download_path}" "${ployzd_url}"; then
        error "Failed to download ployzd binary."
    fi
    tar -xf "${ployzd_download_path}" --directory "${tmp_dir}"
    if ! install "${tmp_dir}/ployzd" "${ployzd_install_path}"; then
        error "Failed to install ployz binary to ${ployzd_install_path}"
    fi
    log "✓ ployzd binary installed: ${ployzd_install_path}"

    log "⏳ Downloading uninstall script: ${uninstall_url}"
    if ! curl -fsSL -o "${uninstall_download_path}" "${uninstall_url}"; then
        error "Failed to download uninstall script."
    fi
    local uninstall_install_path="${INSTALL_BIN_DIR}/ployz-uninstall"
    if ! install "${uninstall_download_path}" "${uninstall_install_path}"; then
        error "Failed to install uninstall.sh script to ${uninstall_install_path}"
    fi
    log "✓ ployz-uninstall script installed: ${uninstall_install_path}"

    # TODO: install ployz CLI binary and create a ployz alias.
}

install_ployz_systemd() {
    local ployz_service_path="${INSTALL_SYSTEMD_DIR}/ployz.service"
    cat > "${ployz_service_path}" << EOF
[Unit]
Description=Ployz machine daemon
After=network-online.target docker.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=${INSTALL_BIN_DIR}/ployzd
TimeoutStartSec=20
Restart=always
RestartSec=2

# Hardening options.
NoNewPrivileges=true
ProtectSystem=full
ProtectControlGroups=true
ProtectHome=read-only
ProtectKernelTunables=true
PrivateTmp=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX AF_NETLINK
RestrictNamespaces=true

[Install]
WantedBy=multi-user.target
EOF
    log "✓ Systemd unit file created: ${ployz_service_path}"


    if [[ "${INSTALL_ONLY}" != "true" ]]; then
        # Reload systemd to recognize the new or updated unit file.
        systemctl daemon-reload
    fi
    systemctl enable ployz.service
}

start_ployz() {
    if [[ "${INSTALL_ONLY}" == "true" ]]; then
        return
    fi

    log "⏳ Starting Ployz machine daemon (ployz.service)..."
    systemctl restart ployz.service
    log "✓ Ployz machine daemon started."
}

log "⏳ Running Ployz install script..."

if [ "$EUID" -ne 0 ]; then
    error "Please run the install script with sudo or as root."
fi

verify_system
install_prerequisites
install_docker
create_ployz_user_and_group
install_ployz_binaries
install_ployz_systemd
start_ployz

# Show warning if Docker was already installed without containerd image store enabled.
if [ "$DOCKER_ALREADY_INSTALLED" = "true" ] && [ "$CONTAINERD_IMAGE_STORE_ENABLED" = "false" ]; then
    echo ""
    warning "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    warning "⚠️  IMPORTANT: Containerd image store configuration"
    warning "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    warning "Docker was already installed on the machine but it doesn't use the containerd"
    warning "image store. Ployz works best with the containerd image store enabled in Docker."
    warning "It allows Docker to directly use the images stored in containerd (pushed with"
    warning "'ployz deploy' or 'ployz image push') without duplicating them in Docker. This saves"
    warning "disk space and makes image management more efficient."
    echo ""
    warning "See https://docs.docker.com/engine/storage/containerd/ for more details."
    echo ""
    warning "To enable it, run the following commands on the machine:"
    echo ""
    echo "sudo bash -c 'cat > ${DOCKER_DAEMON_CONFIG_FILE} << EOF"
    echo "${DOCKER_DAEMON_CONFIG}"
    echo "EOF'"
    echo "sudo systemctl restart docker"
    echo ""
    warning "WARNING: Switching to containerd image store causes you to temporarily lose images"
    warning "and containers created using the classic storage driver. Those resources still"
    warning "exist on your filesystem, and you can retrieve them by turning off the containerd"
    warning "image store feature."
    warning "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
fi

log "✓ Ployz installed on the machine successfully! 🎉"
