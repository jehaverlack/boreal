# Install BOREAL on Linux

BOREAL provides binaries for x86_64, ARM64, and 32-bit ARMv7 Linux systems. It
does not require root privileges and does not modify `PATH`.

## 1. Identify the Processor

```bash
uname -m
```

- `x86_64`: download `boreal-v1.1.1-linux-x86_64`.
- `aarch64` or `arm64`: download `boreal-v1.1.1-linux-aarch64`.
- `armv7l`: download `boreal-v1.1.1-linux-armv7`.

Download the correct file from the [BOREAL download table](../README.md#install-boreal).

## 2. Make the File Executable

The following example uses the x86_64 download:

```bash
cd ~/Downloads
chmod +x boreal-v1.1.1-linux-x86_64
```

Substitute the ARM64 or ARMv7 filename when appropriate.

## 3. Start BOREAL

```bash
./boreal-v1.1.1-linux-x86_64
```

Keep the terminal open while using BOREAL. The application will create its
private runtime directory, install its managed Rclone executable, and open the
local browser interface.

## Verify the Download

```bash
sha256sum boreal-v1.1.1-linux-x86_64
```

Compare the result with the release `SHA256SUMS` file.

## Upgrading

See [Upgrading BOREAL](UPGRADING.md). Quit the running version before starting
the new binary; existing data under `~/.boreal` is retained.
