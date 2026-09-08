# Install BOREAL on macOS

BOREAL supports Apple Silicon and Intel Macs. The beta binaries are not
currently code-signed or notarized, so macOS may block the first launch.

BOREAL does not require administrator privileges for normal operation. Do not
run it with `sudo`.

## 1. Download BOREAL

Use **About This Mac** in the Apple menu to identify the processor:

- Apple M1, M2, M3, or later: download `boreal-v1.1.1-macos-aarch64`.
- Intel processor: download `boreal-v1.1.1-macos-x86_64`.

Download the correct file from the [BOREAL download table](../README.md#install-boreal).

## 2. Make the File Executable

Open Terminal and run the commands for your downloaded file.

Apple Silicon:

```bash
cd ~/Downloads
chmod +x boreal-v1.1.1-macos-aarch64
```

Intel:

```bash
cd ~/Downloads
chmod +x boreal-v1.1.1-macos-x86_64
```

## 3. Start BOREAL

Apple Silicon:

```bash
./boreal-v1.1.1-macos-aarch64
```

Intel:

```bash
./boreal-v1.1.1-macos-x86_64
```

Keep the Terminal window open while using BOREAL. The current beta is a
command-line executable rather than a macOS `.app`, so launching it from Finder
is not supported.

## If macOS Blocks BOREAL

Open **System Settings → Privacy & Security**, find the message that BOREAL was
blocked, and select **Open Anyway**.

If necessary, remove the downloaded file's quarantine attribute and start it
again:

```bash
xattr -d com.apple.quarantine boreal-v1.1.1-macos-aarch64
./boreal-v1.1.1-macos-aarch64
```

Substitute the Intel filename on an Intel Mac. Do not add `sudo`.

## Verify the Download

```bash
shasum -a 256 boreal-v1.1.1-macos-aarch64
```

Compare the result with the release `SHA256SUMS` file. An Apple Silicon binary
can also be checked with `file`; it should be reported as `arm64`.

## Upgrading

See [Upgrading BOREAL](UPGRADING.md). Quit the running version before starting
the new binary; existing data under `~/.boreal` is retained.
