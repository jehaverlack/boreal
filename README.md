# BOREAL

**Browser-based Organizer for Rclone Exploration, Audit & Lookup**

| Attribute | Value |
| --- | --- |
| **Project** | Boreal |
| **Description** | Browser-based Organizer for Rclone Exploration, Audit and Lookup |
| **Author** | John Haverlack |
| **License** | MIT |
| **Version** | 1.1.1 |
| **Maturity** | STABLE |
| **Date** | 2026-09-08 |

> AI Attestation: Generative AI was used in the development of this codebase.
> The architecture and design goals are those of the author.

BOREAL is a local desktop application that inventories and organizes metadata
from the services you choose. Its browser-based dashboard makes large data
collections easier to search, audit, classify, and prepare for migration.

BOREAL currently supports:

- Local files, including duplicate-file analysis
- Persons, groups, organizations, and account relationships
- Google Drive: My Drive, Shared Drives, and Shared with me
- GitHub repository metadata
- S3-compatible object storage
- Keeper shared-folder metadata

BOREAL runs on Linux, Windows, and macOS. Its interface and private inventory
database stay on your computer. It manages its own Rclone installation and
normal use does not require administrator privileges.

## Download BOREAL

Download the binary matching your operating system and processor from the
[BOREAL v1.1.1 release](https://github.com/jehaverlack/boreal/releases/tag/v1.1.1).

| System | Processor | Download | Instructions |
| --- | --- | --- | --- |
| Linux | x86_64 / AMD64 | [Download](https://github.com/jehaverlack/boreal/raw/refs/tags/v1.1.1/dist/boreal-v1.1.1-linux-x86_64) | [Install on Linux](docs/Install-Linux.md) |
| Linux | ARM64 / AArch64 | [Download](https://github.com/jehaverlack/boreal/raw/refs/tags/v1.1.1/dist/boreal-v1.1.1-linux-aarch64) | [Install on Linux](docs/Install-Linux.md) |
| Linux | ARMv7 32-bit | [Download](https://github.com/jehaverlack/boreal/raw/refs/tags/v1.1.1/dist/boreal-v1.1.1-linux-armv7) | [Install on Linux](docs/Install-Linux.md) |
| Windows | x86_64 / AMD64 | [Download](https://github.com/jehaverlack/boreal/raw/refs/tags/v1.1.1/dist/boreal-v1.1.1-windows-x86_64.exe) | [Install on Windows](docs/Install-Windows.md) |
| macOS | Apple Silicon / ARM64 | [Download](https://github.com/jehaverlack/boreal/raw/refs/tags/v1.1.1/dist/boreal-v1.1.1-macos-aarch64) | [Install on macOS](docs/Install-MACOS.md) |
| macOS | Intel x86_64 | [Download](https://github.com/jehaverlack/boreal/raw/refs/tags/v1.1.1/dist/boreal-v1.1.1-macos-x86_64) | [Install on macOS](docs/Install-MACOS.md) |

Verify a download with the release
[SHA256SUMS](https://github.com/jehaverlack/boreal/raw/refs/tags/v1.1.1/dist/SHA256SUMS)
file.

After installation, start BOREAL and open the local address it displays. Choose
at least one module in **App → Settings**; Local Files is a quick first option.
BOREAL downloads Rclone in the background when needed.

## Dashboard

![BOREAL dashboard](docs/boreal-dashboard.png)

## Documentation

- [Installation on Linux](docs/Install-Linux.md), [Windows](docs/Install-Windows.md), or [macOS](docs/Install-MACOS.md)
- [Upgrading BOREAL](docs/UPGRADING.md)
- [Building from source](docs/BUILDING.md)
- [Architecture and security design](docs/DESIGN.md)
- [Roadmap](docs/ROADMAP.md) and [changelog](docs/CHANGELOG.md)

BOREAL never deletes migration sources. Review copied data before manually
changing or removing its source. BOREAL is distributed under the
[MIT License](LICENSE).
