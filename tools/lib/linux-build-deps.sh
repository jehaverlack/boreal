#!/usr/bin/env bash
# Shared, sourceable helpers; sourcing this file never installs packages.

detect_linux_family() {
    local release_file="${1:-/etc/os-release}"
    local ID='' ID_LIKE='' PRETTY_NAME='Linux'
    if [[ ! -r "$release_file" ]]; then
        echo "ERROR: Cannot read $release_file to identify the Linux distribution." >&2
        return 1
    fi
    # os-release is provided by the operating system.
    # shellcheck disable=SC1090
    source "$release_file"
    case " $ID $ID_LIKE " in
        *' debian '*|*' ubuntu '*) LINUX_FAMILY=debian; PACKAGE_MANAGER=apt-get ;;
        *' rhel '*|*' rocky '*|*' almalinux '*|*' centos '*|*' fedora '*) LINUX_FAMILY=rhel; PACKAGE_MANAGER=dnf ;;
        *) echo "ERROR: Unsupported distribution: $PRETTY_NAME. Supported families: Debian/Ubuntu and RHEL/Rocky/AlmaLinux/Fedora." >&2; return 1 ;;
    esac
    LINUX_NAME="$PRETTY_NAME"
}

install_native_dependencies() {
    if [[ "$LINUX_FAMILY" == debian ]]; then
        sudo apt-get update
        sudo apt-get install -y build-essential pkg-config curl ca-certificates jq git perl
    else
        local packages=(gcc gcc-c++ make binutils glibc-devel pkgconf-pkg-config ca-certificates jq git perl)
        # Minimal Rocky/RHEL images may already provide curl via curl-minimal.
        if ! command -v curl >/dev/null 2>&1; then packages+=(curl); fi
        sudo dnf install -y "${packages[@]}"
    fi
}

read_linux_build_targets() {
    local metadata="$1" target setting
    TARGETS=()
    for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu armv7-unknown-linux-gnueabihf x86_64-pc-windows-gnu; do
        if ! jq -e --arg target "$target" '.BUILD_TARGETS[$target] | type == "boolean"' "$metadata" >/dev/null; then
            echo "ERROR: BUILD_TARGETS[\"$target\"] must be true or false in $metadata." >&2
            return 1
        fi
        setting=$(jq -r --arg target "$target" '.BUILD_TARGETS[$target]' "$metadata") || return 1
        case "$setting" in
            true) TARGETS+=("$target") ;;
            false) ;;
            *) echo "ERROR: BUILD_TARGETS[\"$target\"] must be true or false in $metadata." >&2; return 1 ;;
        esac
    done
}

linux_host_target() {
    case "${1:-$(uname -m)}" in
        x86_64) echo x86_64-unknown-linux-gnu ;;
        aarch64|arm64) echo aarch64-unknown-linux-gnu ;;
        armv7l) echo armv7-unknown-linux-gnueabihf ;;
        *) echo "ERROR: Unsupported Linux build architecture." >&2; return 1 ;;
    esac
}

compiler_for_target() {
    local target="$1" host="$2"
    if [[ "$target" == "$host" ]]; then echo cc; return; fi
    case "$target" in
        x86_64-unknown-linux-gnu) echo x86_64-linux-gnu-gcc ;;
        aarch64-unknown-linux-gnu) echo aarch64-linux-gnu-gcc ;;
        armv7-unknown-linux-gnueabihf) echo arm-linux-gnueabihf-gcc ;;
        x86_64-pc-windows-gnu) echo x86_64-w64-mingw32-gcc ;;
        *) return 1 ;;
    esac
}

install_target_compiler() {
    local target="$1" host="$2" compiler package
    compiler=$(compiler_for_target "$target" "$host") || return 1
    if command -v "$compiler" >/dev/null 2>&1; then return; fi
    if [[ "$LINUX_FAMILY" == debian ]]; then
        case "$target" in
            x86_64-unknown-linux-gnu) package=gcc-x86-64-linux-gnu ;;
            aarch64-unknown-linux-gnu) package=gcc-aarch64-linux-gnu ;;
            armv7-unknown-linux-gnueabihf) package=gcc-arm-linux-gnueabihf ;;
            x86_64-pc-windows-gnu) package=gcc-mingw-w64-x86-64 ;;
        esac
    else
        case "$target" in
            x86_64-pc-windows-gnu) package=mingw64-gcc ;;
            # Ask DNF for the exact compiler required, not a different ARM ABI.
            *) package="/usr/bin/$compiler" ;;
        esac
    fi
    if ! sudo "$PACKAGE_MANAGER" install -y "$package"; then
        echo "ERROR: Unable to install $compiler for $target." >&2
        echo "Install a complete target toolchain from your distribution's repositories, or disable this target in metadata.json." >&2
        if [[ "$LINUX_FAMILY" == rhel ]]; then
            echo "Optional cross-compilers may require EPEL and CRB/PowerTools (CodeReady Builder on RHEL). See docs/BUILDING.md." >&2
        fi
        echo "Use --native-only to set up just this machine's native build." >&2
        return 1
    fi
}

verify_target_compiler() {
    local target="$1" host="$2" compiler probe_dir status=0
    compiler=$(compiler_for_target "$target" "$host") || return 1
    probe_dir=$(mktemp -d) || return 1
    # Linking a libc-using program catches incomplete cross GCC packages/sysroots.
    printf '#include <stdio.h>\nint main(void) { puts("BOREAL"); return 0; }\n' |
        "$compiler" -x c -o "$probe_dir/probe" - || status=$?
    rm -f "$probe_dir/probe"
    rmdir "$probe_dir"
    if (( status != 0 )); then
        echo "ERROR: $compiler cannot compile and link a C program for $target." >&2
        echo "Install the target's libc headers, startup objects and libraries as well as GCC. EPEL cross GCC alone may not include a userspace sysroot." >&2
        echo "Alternatively disable $target in metadata.json, or run setup with --native-only." >&2
        return 1
    fi
}
