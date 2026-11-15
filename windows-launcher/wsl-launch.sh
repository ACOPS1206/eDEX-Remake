#!/usr/bin/env bash
# Windows host launcher companion. Invoked inside the Debian WSL2 distro.
set -euo pipefail

: "${EDEX_PACKAGE_ROOT:?Missing converted package path; use eDEX-Remake.exe}"
archive="$EDEX_PACKAGE_ROOT/eDEX-Remake-linux-x86_64-debian13.tar.gz"
if [[ ! -f "$archive" ]]; then
    echo "Missing packaged Debian 13 binary archive." >&2
    exit 1
fi

if [[ "$(uname -m)" != "x86_64" ]]; then
    echo "This package is x86_64 only. Windows ARM64 needs a different Linux binary." >&2
    exit 1
fi

if [[ ! -f /etc/os-release ]]; then
    echo "Cannot identify the WSL Linux distribution." >&2
    exit 1
fi
. /etc/os-release
if [[ "${ID:-}" != "debian" || "${VERSION_ID:-}" != "13" ]]; then
    echo "This WSL package requires Debian 13 (Trixie), got ${PRETTY_NAME:-unknown}." >&2
    echo "Install Debian 13 in WSL or use a build targeting your distro." >&2
    exit 1
fi

install_dir="$HOME/.local/share/eDEX-Remake"
if [[ "${1:-}" == "--setup" ]]; then
    if (( EUID == 0 )); then
        apt-get update
        DEBIAN_FRONTEND=noninteractive apt-get install -y \
            libgtk-4-1 libvte-2.91-gtk4-0 libsdl3-0 fontconfig
    else
        if ! command -v sudo >/dev/null 2>&1; then
            echo "sudo is missing in Debian WSL; install dependencies as root." >&2
            exit 1
        fi
        sudo apt-get update
        sudo apt-get install -y libgtk-4-1 libvte-2.91-gtk4-0 libsdl3-0 fontconfig
    fi
    mkdir -p "$install_dir"
    tar -xzf "$archive" --strip-components=1 -C "$install_dir"
    chmod +x "$install_dir/edex-native"
    echo "eDEX-Remake installed in $install_dir"
    echo "Open eDEX-Remake.exe again to launch it through WSLg."
    exit 0
fi

if [[ $# -gt 0 ]]; then
    echo "Unsupported option: $1" >&2
    exit 2
fi

if [[ ! -x "$install_dir/edex-native" ]]; then
    echo "Run eDEX-Remake.exe --setup before launching." >&2
    exit 1
fi
if [[ -z "${WAYLAND_DISPLAY:-}" && -z "${DISPLAY:-}" ]]; then
    echo "WSLg graphical display is not available. Check WSL2 and run 'wsl --update' on Windows." >&2
    exit 1
fi
if ldd "$install_dir/edex-native" | grep -q 'not found'; then
    echo "Missing Linux runtime libraries. Run eDEX-Remake.exe --setup again." >&2
    exit 1
fi
cd "$install_dir"
exec ./edex-native
