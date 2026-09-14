# Google authentication after Groups removal

Google Groups support and the experimental shared account/token bridge have been
removed. Boreal again uses its existing Rclone remotes for Google authorization.

## Existing installations

Keep the restored `~/.boreal` directory. Boreal reads its existing Desktop client,
`rclone.conf`, database and source settings. No credential conversion is needed.
Retired `google-account.json`, `google-connection.json` and Groups token files are
ignored. Existing database migration history and cached tables are preserved.

Use **Settings → Storage remotes** to reconnect only if Google access needs renewal.
`my-drive-ro` supplies read-only inventory, downloads and private Persons Sheet
access. `my-drive-rw` supplies migration write access and may use another account.
Rclone maintains their tokens independently. Google Groups is absent from Settings,
navigation, API routes, metadata selections and background indexing.

## Fresh installations

Enable Google Drive API in the Boreal project, configure a Desktop OAuth client,
and import its JSON using the Google setup guide. Authorize `my-drive-ro` through
Storage remotes. Authorize `my-drive-rw` only for migrations. There is no Groups
helper, Apps Script deployment or Directory permission setup.

Persons remains an independent local dataset for manual entries and CSV import.
An optional private Persons Sheet requires `my-drive-ro` authorization but does not
require enabling Drive inventory.

## Preserved improvements

The restoration retains browser-based Rclone reconnect fixes (`--auto-confirm`),
sanitized authorization diagnostics, private Desktop client file writes, independent
Settings modals, S3 remote selection, Google menu color/icons, Storage remotes
terminology and existing explorer filtering, tags, sorting and resizing behavior.

Verification covers remote arguments with retired auth files present, client import
without changing remote credentials, settings rendering, schema/settings roundtrip,
and the existing inventory/export/UI tests. Live account consent is not part of
these automated tests.
