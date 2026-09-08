# Building BOREAL

BOREAL's release scripts build versioned binaries from the repository root. Run
them as a regular user. The environment setup scripts install or configure a
per-user Rust toolchain and may ask before changing your shell startup file.

## Linux build environment

The Linux setup script supports Debian and Ubuntu systems with `apt`. It uses
`sudo` to install native build tools, `jq`, and cross-compilers for enabled
Linux and Windows targets. Rust is installed for the current user under
`~/.cargo` and `~/.rustup`.

```bash
./tools/setup-build-linux.sh
```

If the script updates `~/.bashrc`, open a new terminal or reload it:

```bash
source ~/.bashrc
```

## macOS build environment

The macOS setup script requires Apple's Xcode Command Line Tools. If they are
missing, it starts Apple's installer and asks you to rerun the script afterward.
It installs Rust for the current user and installs `jq` under `~/.local/bin`
when needed.

```bash
./tools/setup-build-macos.sh
```

If the script updates your shell configuration, open a new terminal or reload
the file reported by the script before building.

## Select release targets

Release targets are controlled by the Boolean values under `BUILD_TARGETS` in
[`metadata.json`](../metadata.json). Set a target to `true` to include it or
`false` to skip it:

```json
"BUILD_TARGETS": {
  "x86_64-unknown-linux-gnu": true,
  "aarch64-unknown-linux-gnu": false,
  "armv7-unknown-linux-gnueabihf": false,
  "x86_64-pc-windows-gnu": false,
  "x86_64-apple-darwin": true,
  "aarch64-apple-darwin": true
}
```

| Build host | Supported output targets |
| --- | --- |
| Linux | Linux x86_64, Linux ARM64, Linux ARMv7, and Windows x86_64 |
| macOS | macOS Intel x86_64 and macOS Apple Silicon ARM64 |

macOS binaries must be built on macOS. For a multi-platform release, copy the
macOS artifacts into the same `build/` directory used on the Linux release
host.

## Run the release build

```bash
./tools/build-release.sh
```

The script validates `BUILD_TARGETS`, reads the release version from
`METADATA.version`, builds the enabled targets supported by the current host,
and writes versioned binaries to `build/`. It stops with an actionable error
when an enabled Rust target, cross-compiler, or required utility is missing.

For versioning, staging, checksums, and the complete release process, see
[`tools/WORKFLOW.md`](../tools/WORKFLOW.md).
