# Install BOREAL on Windows

BOREAL currently provides a Windows x86_64/AMD64 executable. It does not
require administrator privileges for normal operation and does not modify the
Windows `PATH`.

## 1. Download BOREAL

Download `boreal-v1.1.1-windows-x86_64.exe` from the
[BOREAL download table](../README.md#install-boreal).

## 2. Start BOREAL

1. Open the **Downloads** folder.
2. Double-click `boreal-v1.1.1-windows-x86_64.exe`.
3. Keep the BOREAL console window open while using the application.

Windows may warn that the beta executable is not code-signed. Confirm that the
file came from the official BOREAL repository and verify its checksum before
running it.

## Verify the Download

Open PowerShell in the Downloads directory and run:

```powershell
Get-FileHash .\boreal-v1.1.1-windows-x86_64.exe -Algorithm SHA256
```

Compare the result with the release `SHA256SUMS` file.

## Upgrading

See [Upgrading BOREAL](UPGRADING.md). Quit the running version before replacing
or renaming its executable. BOREAL retains its existing data under the user's
Local App Data directory.
