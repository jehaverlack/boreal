# Roadmap

# Boreal Desktop App

## Feature Requests

- [x] On quit, if jobs are running prompt the users before stopping.
- [ ] Add an Data Retention Date to flag data for removal.
- [x] Add Use cases to About
- [x] Link to local host url if Browser TAB CLOSED
- [x] Executable Icon
- [x] Streamline Client ID Setup
- [x] Separate Google Client ID creation wizard from JSON upload
- [x] Migration Wizard
- [x] new Version Detection
- [x] Add robust logging, Remove startup messages on console.
- [x] Taskbar Menu Icon
- [x] Download as Migrations
- [x] Large Archive Migration - Idempotent Restarts
- [x] Add Google Drive https://drive.google.com/settings/storage
- [x] https://drive.google.com/drive/quota
- [x] Auto close window on shutdown.
- [x] Add Close Console Message
- [ ] Build Org Template in Google Drive for Copying
   -  Support for Org Quotas
   -  Improved People Statuses
- [x] Cleanup Dashboard for modules and colorize modules.
- [x] Cleanup Menu order
- [ ] Deduplication Feature
- [ ] Linux SystemD service
- [ ] Update Documenation
- [ ] Security Docs Sensitivity of Meta Data
- [x] Simplified Readme
- [x] Add a Gear to SEttings Page
- [x] Add icons to Menu Items
- [x] Add S3-compatible object-storage metadata indexing
- [ ] Consider CryptPad as an optional metadata source
- [ ] Consider Nextcloud as an optional metadata source

## Future Implementation: Per-User Installation, Startup, and Updates

Install BOREAL as a per-user application without requiring administrator privileges. Use a stable launcher so operating-system shortcuts and startup registrations do not point directly to a version-specific binary.

### Proposed Layout

```text
BOREAL_HOME/
├── bin/
│   ├── boreal-launcher
│   ├── current.json
│   └── versions/
│       └── <version>/boreal
├── cache/
│   └── updates/
├── conf/
├── data/
└── logs/
```

The stable launcher will select the current version, start BOREAL, apply a staged update during restart, and fall back to the previous version when a new release cannot start successfully.

### Phase 1: Per-User Installation

- [ ] Copy the current executable into `BOREAL_HOME/bin/versions/<version>/`.
- [ ] Add a stable `boreal-launcher` and `current.json` version pointer.
- [ ] Add **Install BOREAL for this user** and installation status under App → Settings.
- [ ] Add an operating-system application launcher:
  - Linux: a desktop entry under `~/.local/share/applications/`.
  - Windows: a per-user Start Menu shortcut.
  - macOS: a signed `BOREAL.app` bundle under the user's Applications directory.
- [ ] Make repeated installation and repair operations idempotent.
- [ ] Provide uninstall controls with an option to retain BOREAL configuration and data.

### Phase 2: Start at Login

- [ ] Add a **Start BOREAL when I sign in** setting.
- [ ] Linux: install and manage a `systemd --user` service under `~/.config/systemd/user/`.
- [ ] Do not enable systemd lingering automatically; expose it only as an advanced option if running after logout is required.
- [ ] Windows: register a per-user Task Scheduler logon task instead of a system service so the tray and browser remain in the interactive session.
- [ ] macOS: use `SMAppService` for the packaged application, with a per-user LaunchAgent as an interim standalone-binary option.
- [ ] Detect, display, repair, enable, disable, start, stop, and restart the platform registration from one cross-platform service interface.

### Phase 3: Safe Staged Updates

- [ ] Extend the release manifest with platform, architecture, file length, SHA-256, and a cryptographic signature.
- [ ] Download new releases into `BOREAL_HOME/cache/updates/` without changing the running binary.
- [ ] Verify the release before moving it into `bin/versions/<version>/`.
- [ ] Record the release as pending and provide a **Restart and update** action.
- [ ] Use the existing active-job safeguard before restarting during metadata updates, migrations, or downloads.
- [ ] Have the stable launcher activate the pending version after BOREAL exits.
- [ ] Perform a startup health check and automatically roll back to the previous version on failure.
- [ ] Retain at least one known-good version and clean up older releases only after successful startup.

### User Experience and Security Requirements

- [ ] Portable BOREAL should continue to work without installation.
- [ ] Installation, startup registration, updates, repair, and uninstall should not require administrator access.
- [ ] Starting BOREAL from the OS application menu should open the running WebUI or start it when necessary.
- [ ] Prevent duplicate backend instances by retaining the existing-instance check.
- [ ] Quote and validate every generated executable and configuration path.
- [ ] Preserve `BOREAL_HOME`, credentials, inventory data, and logs across binary upgrades.
- [ ] Sign/notarize platform releases where supported and never activate an unverified download.

# Boreal Server

This would be a fork of the Boreal desktop app that would provide a serverside app, with OAuth login for clients and connect them to their GDrive for web based Google Drive Audit and Migration Management.

- Google Auth
- Organizational Data Access
- Move from SQLite to Postgress
- Alow User Delegation such that one users can view anothers GDrive content metadata for administration purposes.
