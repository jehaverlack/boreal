#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/linux-build-deps.sh"
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

for distro in debian ubuntu rocky rhel almalinux centos fedora; do
    printf 'ID=%s\nPRETTY_NAME="Test %s"\n' "$distro" "$distro" > "$TEST_DIR/os-release"
    detect_linux_family "$TEST_DIR/os-release"
    case "$distro" in debian|ubuntu) expected=apt-get ;; *) expected=dnf ;; esac
    [[ "$PACKAGE_MANAGER" == "$expected" ]] || fail "manager for $distro"
done
printf 'ID=derivative\nID_LIKE="rhel fedora"\n' > "$TEST_DIR/os-release"
detect_linux_family "$TEST_DIR/os-release"
[[ "$LINUX_FAMILY" == rhel ]] || fail 'ID_LIKE detection'
printf 'ID=alpine\n' > "$TEST_DIR/os-release"
if detect_linux_family "$TEST_DIR/os-release" 2>/dev/null; then fail 'unsupported distro accepted'; fi
if detect_linux_family "$TEST_DIR/missing" 2>/dev/null; then fail 'missing os-release accepted'; fi

for family in debian rhel; do (
    LINUX_FAMILY=$family
    sudo() { printf '%s\n' "$*" >> "$TEST_DIR/$family.commands"; }
    install_native_dependencies
); done
[[ $(cat "$TEST_DIR/debian.commands") == *'apt-get install -y build-essential pkg-config'* ]] || fail 'Debian packages'
[[ $(cat "$TEST_DIR/rhel.commands") == *'dnf install -y gcc gcc-c++ make binutils glibc-devel pkgconf-pkg-config'* ]] || fail 'RHEL packages'
[[ $(cat "$TEST_DIR/rhel.commands") != *'mingw'* ]] || fail 'native setup installs cross-compilers'

jq -n '{BUILD_TARGETS:{"x86_64-unknown-linux-gnu":true,"aarch64-unknown-linux-gnu":true,"armv7-unknown-linux-gnueabihf":false,"x86_64-pc-windows-gnu":false,"x86_64-apple-darwin":true,"aarch64-apple-darwin":true}}' > "$TEST_DIR/metadata.json"
read_linux_build_targets "$TEST_DIR/metadata.json"
[[ "${TARGETS[*]}" == 'x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu' ]] || fail 'enabled target selection or macOS filtering'
jq '.BUILD_TARGETS["aarch64-unknown-linux-gnu"]="false"' "$TEST_DIR/metadata.json" > "$TEST_DIR/invalid.json"
if read_linux_build_targets "$TEST_DIR/invalid.json" 2>/dev/null; then fail 'non-Boolean target accepted'; fi

[[ $(compiler_for_target aarch64-unknown-linux-gnu aarch64-unknown-linux-gnu) == cc ]] || fail 'native ARM compiler'
[[ $(compiler_for_target aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu) == aarch64-linux-gnu-gcc ]] || fail 'ARM cross compiler'
[[ $(linux_host_target x86_64) == x86_64-unknown-linux-gnu ]] || fail 'host target'
for family in debian rhel; do (
    LINUX_FAMILY=$family
    if [[ "$family" == debian ]]; then PACKAGE_MANAGER=apt-get; else PACKAGE_MANAGER=dnf; fi
    command() { if [[ "$1" == -v ]]; then return 1; else builtin command "$@"; fi; }
    sudo() { printf '%s\n' "$*" >> "$TEST_DIR/$family.cross"; }
    install_target_compiler x86_64-pc-windows-gnu x86_64-unknown-linux-gnu
    install_target_compiler armv7-unknown-linux-gnueabihf x86_64-unknown-linux-gnu
); done
[[ $(cat "$TEST_DIR/debian.cross") == *'gcc-mingw-w64-x86-64'* ]] || fail 'Debian MinGW'
[[ $(cat "$TEST_DIR/rhel.cross") == *'mingw64-gcc'* ]] || fail 'RHEL MinGW'
[[ $(cat "$TEST_DIR/rhel.cross") == *'/usr/bin/arm-linux-gnueabihf-gcc'* ]] || fail 'exact ARM hard-float ABI'
(
    LINUX_FAMILY=rhel; PACKAGE_MANAGER=dnf
    command() { return 1; }; sudo() { return 1; }
    if install_target_compiler x86_64-pc-windows-gnu x86_64-unknown-linux-gnu > "$TEST_DIR/failure" 2>&1; then fail 'install failure ignored'; fi
)
[[ $(cat "$TEST_DIR/failure") == *'--native-only'* ]] || fail 'missing cross-compiler recovery guidance'
verify_target_compiler "$(linux_host_target)" "$(linux_host_target)"
(
    compiler_for_target() { echo false; }
    if verify_target_compiler aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu > "$TEST_DIR/sysroot" 2>&1; then fail 'incomplete toolchain accepted'; fi
)
[[ $(cat "$TEST_DIR/sysroot") == *'libc headers'* ]] || fail 'sysroot failure guidance'
bash -n "$SCRIPT_DIR/setup-build-linux.sh" "$SCRIPT_DIR/build-release.sh" "$SCRIPT_DIR/lib/linux-build-deps.sh"
"$SCRIPT_DIR/setup-build-linux.sh" --help >/dev/null
if "$SCRIPT_DIR/setup-build-linux.sh" --bogus >/dev/null 2>&1; then fail 'unknown argument accepted'; fi
echo 'PASS: Debian/RHEL detection, native dependencies, target selection, compiler mapping, failure guidance, and real native C linking'
