# Building BOREAL

BOREAL's release scripts build versioned binaries from the repository root. Run
them as a regular user. The environment setup scripts install or configure a
per-user Rust toolchain and may ask before changing your shell startup file.

## Linux build environment

The Linux setup script detects Debian/Ubuntu (`apt-get`) and RHEL-family
systems (`dnf`), including Rocky Linux, AlmaLinux, CentOS Stream, and Fedora.
Use a regular account with `sudo`; RHEL-family systems need DNF (RHEL/Rocky 8+).
It installs native C/C++ build tools, `pkg-config`, `jq`, Git, and Rust under
the current user's `~/.cargo` and `~/.rustup`.

Only Linux and Windows targets enabled in `metadata.json` receive additional
Rust targets and cross-compilers. macOS targets are handled on macOS. The
script locates `metadata.json` relative to itself, so it can run from another
working directory.

```bash
./tools/setup-build-linux.sh
```

To prepare only the native build environment:

```bash
./tools/setup-build-linux.sh --native-only
```

This option does not edit `metadata.json`. The release script still builds
the targets enabled there. Both scripts select the native C compiler for a
matching host architecture, including ARM hosts.

Optional cross-compilers on Rocky/RHEL may require EPEL and CRB/PowerTools
(CodeReady Builder on RHEL). Setup uses your enabled repositories; it reports
missing packages with recovery instructions. Follow the official
[Rocky repository instructions](https://wiki.rockylinux.org/rocky/repo/) or
[EPEL getting-started instructions](https://docs.fedoraproject.org/en-US/epel/getting-started/)
for your distribution before enabling additional targets.

Windows uses `gcc-mingw-w64-x86-64` on Debian/Ubuntu and `mingw64-gcc` on
RPM-based systems. Linux cross-builds need a complete userspace toolchain:
GCC, matching target libc headers, startup objects, and libraries. Some EPEL
cross-GCC packages contain only the compiler. Setup compiles and links a small
C program to detect missing sysroots before declaring success. Supply a
complete toolchain, disable the unavailable target, or use `--native-only`.

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
