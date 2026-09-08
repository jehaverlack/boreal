| Attribute | Value |
| --- | --- |
| **Name** | boreal |
| **Author** | John Haverlack |
| **License** | MIT |
| **Version** | 1.1.1 |
| **Maturity** | STABLE |
| **Date** | 2026-09-08 |

# BOREAL design

## Purpose

BOREAL—the Browser-based Organizer for Rclone Exploration, Audit & Lookup—is a local desktop application for examining Google Drive content through rclone and optionally inventorying GitHub repositories and Keeper shared-folder access. It was developed at the Alaska Center for Energy and Power (ACEP), University of Alaska Fairbanks (UAF), to help organizations identify content at risk of loss, quota pressure, inappropriate continued access, or missing stewardship.

The primary use cases are:

- Inventory My Drive, Shared Drives, and Shared with me content.
- Search and browse Drive content by hierarchy, size, age, owner, permission, and local tag.
- Correlate Drive owners and permissions with an organizational identity directory.
- Identify content owned by departing, former, external, or unknown identities.
- Identify permissions held by people or accounts that should be reviewed.
- Prepare content for migration from user-owned storage to organization-owned Shared Drives.
- Retain a historical local record when an item disappears or Shared Drive access is lost.
- Inventory repository-level GitHub metadata without reading repository contents.
- Review Keeper shared-folder membership and management responsibility without importing records or secrets.

BOREAL is an audit and decision-support tool. It is not a Google Drive replacement, backup system, identity provider, or records-management authority.

## Design criteria

### Safety first

- The default and currently configured Drive integration is the `my-drive-ro` rclone remote using the `drive.readonly` scope.
- Initial setup must not request write access.
- Metadata collection must never modify Google Drive content or permissions.
- Local tags change only SQLite records; they do not modify Google Drive labels or metadata.
- Deletion remains a manual action in Google Drive.
- Migration copies may write only through the separately authorized `my-drive-rw` remote to an explicitly selected Google Drive destination. Local downloads write only to a user-selected directory. Neither operation deletes a source.
- A future automated workflow may apply `Safe for removal` only after a completed migration comparison satisfies the defined verification policy. The current manually applied tag is advisory and is not proof that verification occurred.

### Local operation and privacy

- The web server must bind only to a loopback address (`127.0.0.1`, `localhost`, or `::1`).
- Application state, configuration, logs, credentials, cache files, and SQLite data remain under the user-specific BOREAL home.
- OAuth tokens and client secrets must not be exposed through status pages or logs.
- On Unix-like systems, secret and rclone configuration files are restricted to mode `0600`.
- BOREAL does not require a hosted service or central database.

### Auditability and preservation

- Drive objects are keyed by Google Drive item ID within an inventory scope, not by display name or path.
- Synchronization is idempotent: an item seen again is updated rather than duplicated.
- Items missing from a completed scope scan are soft-deleted, retaining their last metadata and deletion timestamp.
- Shared Drives missing from discovery are retained and marked inaccessible rather than removed.
- Scan and directory-import runs retain timestamps, counts, status, and error information.
- Significant metadata activity and failures are written to the current daily log.

### Accuracy

- My Drive, Shared Drives, and Shared with me are separate inventory scopes and must never be combined in stored or displayed totals.
- Dashboard statistics use only non-deleted items from the relevant scope.
- Shared Drive aggregate totals include only currently accessible Shared Drives.
- Folder size is derived from indexed descendant file sizes and is a logical inventory total, not a Google quota measurement.
- Google-native documents may not report conventional byte sizes or checksums; comparisons must account for that limitation.
- Permissions and ownership describe the latest completed scan, not necessarily Google’s current state between scans.

### Incremental delivery

- Prefer small, independently verifiable features over a single broad migration implementation.
- Preserve existing databases through ordered, forward-only SQL migrations.
- Separate implemented behavior from proposed behavior in UI copy and documentation.
- Keep migration planning, copying, verification, and removal eligibility as distinct states.

### Responsiveness

- Network and large SQLite work must run outside the asynchronous web request path where practical.
- Metadata jobs run in a blocking worker and expose progress through polled UI fragments.
- The application permits only one metadata job and one remote-setup job at a time.
- Explorer queries should remain scoped to the selected directory. Pagination and batched permission/tag loading are expected future optimizations for large folders.

## System context

```text
Google Drive / Google Workspace ── OAuth through rclone ──┐
GitHub ── fine-grained read-only access tokens ───────────┤
Keeper ── authenticated Commander metadata report ────────┤
                                                         ▼
   BOREAL Rust process
   ├── bootstrap and configuration
   ├── application state and background jobs
   ├── rclone process integration
   ├── SQLite inventory and directory index
   └── local Axum/Askama web interface
          │
          ▼
  User's local web browser
```

BOREAL is distributed as a standalone Rust executable. It installs or reuses a BOREAL-managed rclone binary, starts the rclone WebGUI/remote-control process, starts its own local web server, opens the dashboard in the default browser, and remains available through a native desktop tray or menu-bar icon.

## Runtime architecture

### Startup sequence

1. `bootstrap::initialize` resolves the platform-specific BOREAL home.
2. Embedded configuration templates are created only when files do not already exist.
3. Configured directories are resolved and created.
4. Daily file logging is initialized.
5. BOREAL probes its configured local endpoint. A second launch opens the existing WebUI and exits without starting another server.
6. SQLite is opened and pending migrations are applied.
7. The configured Google OAuth client is detected.
8. The native desktop tray or menu-bar controller is registered.
9. The managed rclone binary is installed or validated.
10. The rclone GUI/remote-control process is started.
11. Configured Google remotes are inspected without displaying credentials.
12. Axum binds the local-only web interface and opens the dashboard.
13. Ctrl-C, the WebUI quit action, or the desktop Quit menu triggers graceful shutdown and stops the managed rclone process. Interactive quit actions request confirmation when metadata updates, migrations, or downloads are active.

### Application state

`AppState` is shared through `Arc<AppState>` and contains:

- Resolved runtime paths and configuration.
- SQLite availability.
- Rclone installation/process state.
- Google OAuth client state.
- Read-only inventory and read/write migration remote states.
- Metadata synchronization and progress state.
- Local download status and persistent migration job state.
- Mutual-exclusion guards for metadata and remote-setup jobs.
- A shutdown notification channel.

Mutable state uses `RwLock` for observable status and `Mutex` for exclusive job/process ownership. Long-running rclone, network, parsing, and synchronization work is executed with `spawn_blocking` so the web server can continue serving status and progress requests.

### Source layout

| Area | Responsibility |
| --- | --- |
| `src/main.rs` | Process entry point, logging macros, startup, and shutdown coordination |
| `src/bootstrap.rs` | BOREAL home, embedded templates, directories, and file protection |
| `src/config.rs` | JSON configuration loading, saving, and directory resolution |
| `src/app.rs` | Shared state, initialization, remote setup, and metadata job orchestration |
| `src/rclone/` | Rclone installation, configuration, command execution, GUI, identity, and inventories |
| `src/google/` | Google OAuth client credential detection and import |
| `src/github/` | Optional GitHub credentials and repository metadata client |
| `src/keeper/` | Optional Keeper Commander execution and allowlisted shared-folder report parsing |
| `src/database/` | SQLite initialization, migrations, inventory, directory, tags, and settings |
| `src/web/` | Local Axum server, routes, view models, form handling, and Askama rendering |
| `tmpl/html/` | Embedded HTML pages and polled fragments |
| `tmpl/conf/` | Initial non-secret and secret configuration templates |
| `docs/` | Design and operational documentation |

## Filesystem and configuration

Default locations are:

| Name | Linux/macOS default | Purpose |
| --- | --- | --- |
| `HOME` | `~/.boreal` | BOREAL runtime root |
| `BIN` | `~/.boreal/bin` | Managed rclone and optional user-installed helper executables |
| `CONF` | `~/.boreal/conf` | Application, Google, rclone, GitHub, and Keeper configuration and credentials |
| `DATA` | `~/.boreal/data` | Persistent application data |
| `CACHE` | `~/.boreal/data/cache` | Temporary rclone inventory files |
| `SQLITE` | `~/.boreal/data/sqlite` | `boreal.sqlite` and SQLite sidecar files |
| `LOGS` | `~/.boreal/logs` | Daily `YYYY-MM-DD.boreal.log` files |

`boreal.json` defines the directory graph and WebUI settings. Directory values may refer to other configured directories using uppercase pseudo-variables such as `HOME`, `DATA`, and `SQLITE`. Existing user configuration files are never overwritten by embedded defaults.

## Google and rclone integration

### OAuth client

The user supplies a Google OAuth desktop client configuration. BOREAL imports and stores the client details locally. This client is used by rclone’s browser-based authorization flow.

### Remote convention

| Remote | Scope | Current role |
| --- | --- | --- |
| `my-drive-ro` | `drive.readonly` | Required inventory and identity source |
| `my-drive-rw` | `drive` | Optional, separately authorized destination access for Google Drive migrations |

All metadata queries must use `my-drive-ro`. Shared with me and Shared Drive inventories are views reached through rclone flags on that same authenticated remote; they are not separate OAuth remotes.

BOREAL treats a remote with an unexpected backend, client ID, or OAuth scope as a conflict rather than silently changing it.

### Inventory commands

BOREAL invokes `rclone lsjson` recursively with metadata, owner information, permissions, and `--fast-list`. Source-specific behavior is selected with:

- No additional source flag for My Drive.
- `--drive-shared-with-me` for Shared with me.
- `--drive-team-drive <drive-id>` for an individual Shared Drive.

Shared Drives are discovered using the rclone Drive backend’s `drives` command. Rclone JSON output is written to the cache directory, parsed into memory, validated for Drive IDs, and removed after parsing.

## Metadata update lifecycle

### Local Files inventory

Local Files is an optional, read-only metadata source configured in Settings. BOREAL recursively scans one or more absolute local folders without following symbolic links. Hidden entries, common cache directories, temporary/trash directories, and user-defined wildcard patterns can be excluded. BOREAL's own runtime directory is always excluded.

The SQLite inventory records names, relative paths, sizes, modification times, and SHA-256 checksums. To limit disk I/O, BOREAL hashes only non-empty files whose sizes match another inventory candidate and reuses a checksum while size and modification time remain unchanged. Equal SHA-256 values identify exact duplicate groups. BOREAL does not change or delete local files.

Updates are manual. BOREAL does not automatically query Google Drive at startup or on a schedule.

The Update dialog permits any combination of:

- My Drive
- Shared Drives
- Shared with me
- Persons directory
- GitHub repositories, when configured
- Keeper shared folders, when configured

At least one source is required. Persons is selectable only when an enabled spreadsheet source URL exists. Unselected inventories and timestamps remain unchanged.

The current job performs selected work sequentially in a background worker. The modal displays a separate progress row for each source, marks unselected sources as not requested, and shows typical duration guidance. Rclone does not provide an exact total-item download progress value, so displayed percentages and completion times are estimates.

Only one metadata update may run at once. Failures are logged and represented in scan/import state. Successful completion causes dashboard/status fragments to refresh without requiring a full page reload.

Google work uses rclone and the read-only remote. GitHub uses separately stored fine-grained tokens. Keeper runs the configured Keeper Commander executable with BOREAL's private Keeper configuration and parses `share-report --folders --format json`. GitHub-only and Keeper-only updates do not require rclone to be ready.

## Inventory scopes and reconciliation

| Logical source | Stored scope |
| --- | --- |
| My Drive | `my-drive-ro` |
| Shared with me | `shared-with-me` |
| Shared Drive | `shared-drive:<google-drive-id>` |

Each scope is synchronized independently:

1. De-duplicate the rclone response by Google Drive item ID.
2. Insert new items or update existing items using `(remote_name, item_id)`.
3. Replace normalized permissions when permission scanning is enabled.
4. Calculate cumulative descendant size for folders.
5. Mark previously active items missing from the completed scan as deleted.
6. Complete the scan run with files, folders, permissions, bytes, and deletion counts.

Deleted rows remain queryable through the Explorer’s “Include deleted items” option. A later scan that sees an item again clears its deleted state.

Shared Drive discovery retains Drives that are no longer returned, setting `is_accessible = false`. The Shared Drives page hides these by default and offers “Show inaccessible Shared Drives” to browse their historical inventory. A targeted future per-Drive update must not mark unselected Drives inaccessible; discovery and inventory selection must remain separate operations.

## Persistence architecture

SQLite is the local system of record for indexed metadata and user annotations. Connections are short-lived, foreign keys are enabled, write-ahead logging is used, and writes for a synchronization/import are grouped into transactions.

### Core table groups

| Group | Tables | Purpose |
| --- | --- | --- |
| Configuration | `settings`, `schema_migrations` | Runtime preferences and schema versioning |
| Scan history | `scan_runs` | Scope status, timing, counts, and errors |
| Drive inventory | `drive_items`, `drive_permissions`, `shared_drives`, `shared_drive_permissions` | Current and retained historical Drive state |
| Content classification | `tags`, `drive_item_tags`, `shared_drive_tags` | Local item and Shared Drive tags, including recursive folder tagging |
| Identity directory | `principals`, `principal_emails`, `organizations` | People, groups, department accounts, service accounts, and organizations |
| Relationships | `organization_memberships`, `principal_memberships` | Principal-to-organization and group membership relationships |
| Identity classification | `principal_tags` | Local tags applied to directory identities |
| Directory ingestion | `directory_sources`, `directory_import_runs` | CSV/Google Sheet source configuration and import history |
| Authentication identity | `remote_accounts` | Detected account associated with an rclone remote |
| Migration history | `migration_jobs`, `migration_sources` | Persistent migration/download plans, destinations, progress, retries, and errors |
| GitHub inventory | `github_organizations`, `github_repositories`, `github_repository_tags` | Optional repository-level metadata and local tags |
| Keeper inventory | `keeper_shared_folders`, `keeper_shared_folder_access`, `keeper_shared_folder_tags` | Optional shared-folder access metadata and local tags |

### Storage considerations

The current schema retains both normalized fields and raw metadata/permission JSON. This supports future interpretation but increases database size. Permission-heavy inventories can therefore produce large SQLite files. Future performance work should measure table/index size and query plans before removing raw data.

SQLite is suitable for a single-user local application of this scale. The more immediate responsiveness risks are unbounded Explorer result sets, correlated permission/tag queries, and large rendered permission lists. Planned optimizations include server-side pagination, batched related-row loading, filtered SQL summaries, slow-query logging, and moving all expensive request-time queries to blocking workers.

## Identity directory

The directory correlates Drive email identities with organizational context. A principal can represent a person, Google group, department account (`dept_acct`), service account (`service_acct`), or another imported/custom type. Principal type is intentionally open-ended rather than a fixed database enum.

Directory data may be:

- Imported from an uploaded CSV.
- Imported from a configured Google Sheet CSV URL during a selected metadata update.
- Added or edited manually.

The expected import fields are `name`, `email`, `organization`, `type`, `status`, `departure_date`, and `notes`. Email matching is case-insensitive. Re-import updates matching principals. For imported organization data, the source row is authoritative for the principal’s organization rather than additive.

Identity tags can be used independently for owner and permission filtering. Explorer identity pills indicate known identities, show assigned tag colors, and use a dashed amber border for identities missing from the directory. Clicking an unknown identity opens the add-directory-entry workflow with the email prefilled.

The configured Google Sheet must be readable by the authenticated account. Viewer access through direct membership or an accessible Google group is sufficient only when Google permits CSV export/download for that account and Shared Drive policy.

## Local tags

Default content tags are:

- `To Migrate`
- `To Delete`
- `To Export`
- `Safe for removal`

Users can create and edit custom tags, descriptions, colors, and applicable scopes. Tags may be applied to Persons, Shared Drives themselves, selected Drive items, GitHub repositories, or Keeper shared folders when the tag supports that scope. Applying a tag to a Drive folder recursively applies it to indexed descendants in the same scope. GitHub and Keeper tags affect only the selected metadata rows. Removing tags is also a local SQLite operation.

`Safe for removal` is intended to mean that a migration was reviewed and the source can be considered for manual removal. Today it can be applied manually, so BOREAL does not treat the tag itself as proof that verification occurred. It is advisory and does not remove, trash, or modify the Google Drive source.

## Web interface

Axum provides local routes and Askama renders HTML at compile time. Bootstrap supplies layout and components. Lightweight JavaScript and fragment polling update status, setup, metadata progress, and dashboard summaries.

Primary views are:

- Dashboard summaries for My Drive, Shared Drives, and Shared with me, plus conditional GitHub and Keeper summaries.
- My Drive Explorer.
- Shared Drives list and per-Drive Explorer.
- Shared with me Explorer.
- Optional GitHub repository Explorer.
- Optional Keeper shared-folder access Explorer.
- Migration and local-download planning, progress, resume, and history views.
- Directory list, detail, add, and edit pages.
- Tag management.
- Remotes status.
- Settings.
- About.

Explorer behavior includes hierarchy navigation, Google Drive links, sortable columns, per-column filters, deleted-item visibility, selection, content tags, identity-tag filters, permission pills, and filtered-result summaries. Folder names navigate within Boreal; the adjacent Google Drive icon opens the underlying Drive object.

Shared Drives do not have individual owners, so the list shows Managers instead. During permission-enabled updates, BOREAL queries each Shared Drive root separately and records identities with Google's `organizer` role as Managers. The Permissions column combines those Drive-root identities with users, groups, domains, and other principals observed in indexed item permissions; observed roles are available on hover. This remains an audit view and may be constrained by the authenticated account's visibility.

The status bar reports rclone, authenticated Google account, client configuration, remote count, metadata age/state, active migrations, downloads, and BOREAL version. Metadata age uses green, amber, and red states based on freshness thresholds.

## Security and trust boundaries

### Trusted local components

- The BOREAL executable and embedded templates.
- The BOREAL-managed rclone executable after installation.
- Files inside the user’s BOREAL home, subject to local account security.
- The local browser session on the same workstation.

### External/untrusted inputs

- Google OAuth and Drive API responses.
- Rclone command output.
- Uploaded CSV files.
- Linked Google Sheet CSV content.
- GitHub API responses.
- Keeper Commander output.
- User-entered URLs, filters, tag names, colors, and directory values.

External data must be parsed, validated, escaped by Askama, and written using SQL parameters. Drive IDs are authoritative identifiers. URLs must be parsed to IDs rather than used directly in shell commands. Child process execution uses argument arrays rather than a shell command string.

GitHub token files and Keeper Commander configuration are private local credentials and must never be rendered, logged, or copied into inventory tables. Keeper report parsing uses an allowlisted model containing only folder UID, name, type, path, sharing target, and permission labels; unknown JSON fields are ignored.

### Known security boundary

Loopback binding prevents network exposure but does not provide browser authentication or CSRF protection. The current model assumes a single trusted user and workstation. If BOREAL ever binds beyond loopback or becomes multi-user, authentication, authorization, CSRF protection, session isolation, and stronger secret storage become mandatory architectural requirements.

## Migration and download scope

### Current status

BOREAL can create persistent jobs from selected My Drive or Shared with me items. A job can copy to a user-selected local directory or, after the optional `my-drive-rw` remote is authorized, to a validated Google Drive folder. Entire accessible Shared Drives can also be downloaded locally.

The first copy rejects destination name collisions. An interrupted or failed job can be resumed as a one-way synchronization: missing and changed destination files are updated, destination-only files are retained, and source content is never modified or deleted. Job records retain source totals, destination, progress, status, errors, and resume count.

Destination inventory comparison, checksum-based verification, permission comparison, and automatic `Safe for removal` classification are not yet implemented. Users must verify completed copies themselves; a `copied` job status records transfer completion, not policy-grade verification.

### Migration boundary

The migration workflow does not create Shared Drives or destination folders. The user creates and configures the destination in Google Drive and supplies its URL to BOREAL, or selects an existing local directory for a download.

The implemented workflow is:

1. Select a My Drive or Shared with me file/folder.
2. Supply an existing destination Shared Drive or folder URL.
3. Validate the destination URL and access, or select a local directory.
4. Record the source inventory totals in a persistent job.
5. For Google copies, use the separately authorized read/write remote.
6. Preflight the destination and copy the source without changing it.
7. Retain progress and errors so interrupted work can be resumed.
8. Require the user to verify the destination and leave source removal to the user in Google Drive.

For Shared with me content, BOREAL copies readable content but does not attempt to delete an owner’s original. The original owner and source availability remain part of the migration report.

### Current non-goals

- Creating or deleting Shared Drives.
- Automatically deleting, trashing, or moving My Drive sources.
- Permanently deleting any Google Drive item.
- Guaranteeing exact replication of permissions that conflict with Shared Drive policy.
- Treating path/name equality alone as proof of a correct copy.
- Treating a completed copy status as proof of structural or permission equivalence.

### Planned verification criteria

A migration may be considered structurally verified when:

- Every expected relative path has a corresponding destination item.
- File and folder counts match under the selected roots.
- Logical size totals match within rules for Google-native documents.
- Available checksums match for binary files.
- No item is failed, skipped, inaccessible, or unresolved unless explicitly accepted by policy.
- Shortcut handling is reported and consistent with the selected copy policy.
- Permission differences are either resolved or explicitly acknowledged according to policy.
- A destination inventory completed after the copy.

Verification results and exceptions must be retained as a migration audit record before automatic application of `Safe for removal` is introduced.

## Planned evolution

Near-term work should proceed in this order:

1. Improve Explorer responsiveness through pagination, batched permission/tag loading, and query timing.
2. Separate discovery from inventory selection and permit updates of selected Shared Drives.
3. Make per-source update timing persist at actual source start/finish boundaries.
4. Add destination refresh and structural comparison records to existing migration jobs.
5. Add checksum-aware comparison and explicit exception handling.
6. Add permission comparison and policy review.
7. Apply `Safe for removal` automatically only after verified completion.
8. Add signed, staged application updates through a stable per-user launcher.

Any feature that expands Google permissions, changes remote content, or communicates outside the local machine requires an explicit design and safety review before implementation.
