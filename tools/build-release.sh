#!/usr/bin/env bash
set -euo pipefail

# -------------------------------------------------
# Resolve project root
# -------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

# -------------------------------------------------
# Application
# -------------------------------------------------

APP="boreal"
OUT="build"

# -------------------------------------------------
# Rust targets
# -------------------------------------------------

LINUX_X86_64="x86_64-unknown-linux-gnu"
LINUX_AARCH64="aarch64-unknown-linux-gnu"
LINUX_ARMV7="armv7-unknown-linux-gnueabihf"

WINDOWS_X86_64="x86_64-pc-windows-gnu"

DARWIN_X86_64="x86_64-apple-darwin"
DARWIN_AARCH64="aarch64-apple-darwin"

# -------------------------------------------------
# Helpers
# -------------------------------------------------

check_target() {
    local target="$1"

    if ! rustup target list --installed | grep -qx "$target"; then
        echo "ERROR: Rust target not installed: $target"
        echo
        echo "Install with:"
        echo "  rustup target add $target"
        exit 1
    fi
}

check_command() {
    local command="$1"
    local hint="$2"

    if ! command -v "$command" >/dev/null 2>&1; then
        echo "ERROR: Required command not found: $command"
        echo
        echo "$hint"
        exit 1
    fi
}

target_enabled() {
    local target="$1"

    jq -e \
        --arg target "$target" \
        '.BUILD_TARGETS[$target] == true' \
        metadata.json \
        >/dev/null
}

validate_target_setting() {
    local target="$1"
    local setting

    setting="$(
        jq -r \
            --arg target "$target" \
            '.BUILD_TARGETS[$target] | type' \
            metadata.json
    )"

    if [[ "$setting" != "boolean" ]]; then
        echo "ERROR: BUILD_TARGETS[\"$target\"] must be true or false"
        exit 1
    fi
}

# -------------------------------------------------
# Prerequisites
# -------------------------------------------------

echo "==> BOREAL release build"

check_command jq "Install jq before building."

VERSION="$(jq -r '.METADATA.version' metadata.json)"

if [[ -z "$VERSION" || "$VERSION" == "null" ]]; then
    echo "ERROR: Unable to read METADATA.version from metadata.json"
    exit 1
fi

for target in \
    "$LINUX_X86_64" \
    "$LINUX_AARCH64" \
    "$LINUX_ARMV7" \
    "$WINDOWS_X86_64" \
    "$DARWIN_X86_64" \
    "$DARWIN_AARCH64"
do
    validate_target_setting "$target"
done

HOST_OS="$(uname -s)"

echo "==> Version: v${VERSION}"
echo "==> Build host: ${HOST_OS}"

mkdir -p "$OUT"

# -------------------------------------------------
# Linux build host
# -------------------------------------------------

if [[ "$HOST_OS" == "Linux" ]]; then

    echo
    echo "==> Checking Linux/Windows Rust targets"

    if target_enabled "$LINUX_X86_64"; then
        check_target "$LINUX_X86_64"
    fi

    if target_enabled "$LINUX_AARCH64"; then
        check_target "$LINUX_AARCH64"
    fi

    if target_enabled "$LINUX_ARMV7"; then
        check_target "$LINUX_ARMV7"
    fi

    if target_enabled "$WINDOWS_X86_64"; then
        check_target "$WINDOWS_X86_64"
    fi

    echo "==> Checking target C compilers"
    # shellcheck source=lib/linux-build-deps.sh
    source "$SCRIPT_DIR/lib/linux-build-deps.sh"
    host_target=$(linux_host_target)
    for target in "$LINUX_X86_64" "$LINUX_AARCH64" "$LINUX_ARMV7" "$WINDOWS_X86_64"; do
        if target_enabled "$target"; then
            compiler=$(compiler_for_target "$target" "$host_target")
            check_command "$compiler" "Run ./tools/setup-build-linux.sh; see docs/BUILDING.md for distribution-specific cross-compilers."
            # Use the native compiler on ARM hosts too; override the repository's
            # cross-linker defaults only for this build process.
            target_key=${target//-/_}
            export "CARGO_TARGET_${target_key^^}_LINKER=$compiler"
            export "CC_${target_key}=$compiler"
        fi
    done

    # ---------------------------------------------
    # Linux x86_64
    # ---------------------------------------------

    if target_enabled "$LINUX_X86_64"; then
        echo
        echo "==> Building Linux x86_64"

        cargo build \
            --release \
            --target "$LINUX_X86_64"

        cp \
            "target/$LINUX_X86_64/release/$APP" \
            "$OUT/${APP}-v${VERSION}-linux-x86_64"
    fi

    # ---------------------------------------------
    # Linux ARM64
    # ---------------------------------------------

    if target_enabled "$LINUX_AARCH64"; then
        echo
        echo "==> Building Linux ARM64"

        cargo build \
            --release \
            --target "$LINUX_AARCH64"

        cp \
            "target/$LINUX_AARCH64/release/$APP" \
            "$OUT/${APP}-v${VERSION}-linux-aarch64"
    fi

    # ---------------------------------------------
    # Linux ARMv7
    # ---------------------------------------------

    if target_enabled "$LINUX_ARMV7"; then
        echo
        echo "==> Building Linux ARMv7"

        cargo build \
            --release \
            --target "$LINUX_ARMV7"

        cp \
            "target/$LINUX_ARMV7/release/$APP" \
            "$OUT/${APP}-v${VERSION}-linux-armv7"
    fi

    # ---------------------------------------------
    # Windows x86_64
    # ---------------------------------------------

    if target_enabled "$WINDOWS_X86_64"; then
        echo
        echo "==> Building Windows x86_64"

        cargo build \
            --release \
            --target "$WINDOWS_X86_64"

        cp \
            "target/$WINDOWS_X86_64/release/$APP.exe" \
            "$OUT/${APP}-v${VERSION}-windows-x86_64.exe"
    fi

    # ---------------------------------------------
    # Darwin notice
    # ---------------------------------------------

    if target_enabled "$DARWIN_X86_64" || target_enabled "$DARWIN_AARCH64"; then
        echo
        echo "==> Enabled macOS builds require a macOS host:"

        if target_enabled "$DARWIN_X86_64"; then
            echo "      $DARWIN_X86_64"
        fi

        if target_enabled "$DARWIN_AARCH64"; then
            echo "      $DARWIN_AARCH64"
        fi
    fi

# -------------------------------------------------
# macOS build host
# -------------------------------------------------

elif [[ "$HOST_OS" == "Darwin" ]]; then

    echo
    echo "==> Checking macOS Rust targets"

    if target_enabled "$DARWIN_X86_64"; then
        check_target "$DARWIN_X86_64"
    fi

    if target_enabled "$DARWIN_AARCH64"; then
        check_target "$DARWIN_AARCH64"
    fi

    # ---------------------------------------------
    # macOS Intel
    # ---------------------------------------------

    if target_enabled "$DARWIN_X86_64"; then
        echo
        echo "==> Building macOS x86_64"

        cargo build \
            --release \
            --target "$DARWIN_X86_64"

        cp \
            "target/$DARWIN_X86_64/release/$APP" \
            "$OUT/${APP}-v${VERSION}-macos-x86_64"
    fi

    # ---------------------------------------------
    # macOS Apple Silicon
    # ---------------------------------------------

    if target_enabled "$DARWIN_AARCH64"; then
        echo
        echo "==> Building macOS ARM64"

        cargo build \
            --release \
            --target "$DARWIN_AARCH64"

        cp \
            "target/$DARWIN_AARCH64/release/$APP" \
            "$OUT/${APP}-v${VERSION}-macos-aarch64"
    fi

else
    echo "ERROR: Unsupported build host: $HOST_OS"
    exit 1
fi

# -------------------------------------------------
# Results
# -------------------------------------------------

echo
echo "==> Build complete"
echo "==> Version: v${VERSION}"
echo
ls -lh "$OUT"
