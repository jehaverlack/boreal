# Shared Google authentication migration

The new Google connection replaces separate sign-ins for Drive inventory, Drive
migration operations and Groups. Private Persons Sheets use the same Google Drive
read grant; they do not need public sharing or the Drive inventory dataset enabled.

## Configure an installation

Open **Settings → Google connection**. Import your project's Desktop OAuth JSON or
a Boreal Google setup profile, choose services, supply the private Sheet URL if used,
and configure the My Groups helper deployment ID. Select migration write access only
if needed. Click **Connect Google and check access**, then approve Google consent.
Each service is checked independently; review any unsuccessful checks before selecting
it in Update. Check access does not modify an inventory.

For shared deployments, follow [the helper deployment instructions](../tools/google-groups/README.md)
once, then distribute the reusable project profile. Users do not configure separate
Rclone authorizations or receive administrative roles for My Groups. Additional
storage accounts and legacy Rclone configuration remain in the advanced connection
manager.

## Compatibility and privacy

- Legacy `rclone.conf` and `google-groups-token.json` are retained. Working legacy
  Drive connections continue to work until a shared connection is created.
- `google-account.json` owns the new account identity, granted scopes and offline
  authorization. `google-connection.json` stores deployment and optional capability
  choices. On Unix, newly written Google credential files have mode 0600 and are
  replaced atomically. Credentials never enter inventory or export reports.
- Managed Google operations then use the shared account exclusively. Missing scopes
  must not silently fall back to a different legacy account. Additional user-managed
  storage remotes remain unchanged.
- Rclone receives a temporary private configuration and an ephemeral loopback token
  endpoint. Only Boreal exchanges the Google refresh token. Rclone can renew access
  during long jobs; its temporary token contains a local capability rather than the
  Google refresh token. The bridge stops and deletes its configuration after the job.
- Existing Groups inventory remains stored under its account. Switching from
  Directory IDs to helper IDs preserves existing group links by matching email.
  Failed or malformed snapshots cannot delete an earlier successful inventory.
- Drive read permission includes file content because Boreal supports downloads and
  authenticated Sheet CSV export. A shared account with migration permission has a
  write-capable Google grant even when an inventory job only reads. Disabling
  migration operations blocks those operations locally; disabling a service does
  not revoke previously granted Google authorization.
- New permissions, revocation and organization policies can require consent again.
  Routine updates refresh access automatically. External OAuth Testing mode is not
  suitable for durable installations because its refresh tokens expire in seven days.

## Validation and release status

This is a candidate for Boreal 2.0 because the owner and lifecycle of Google
credentials changed. Release numbering remains 1.2.0 until a release is prepared;
this work does not publish a stable 2.0 release.

Run the Rust suite, `node tools/test-google-groups-helper.cjs`, and
`node tools/test-web-controls.cjs`. To include the real Rclone token bridge fixture,
set `BOREAL_TEST_RCLONE` to the managed executable when running `cargo test`. That
fixture uses synthetic credentials and a rejecting proxy, so it cannot query Google.
Set `BOREAL_UI_FIXTURE_DIR` to render Settings and Google connection HTML fixtures.

Local checks cover scope selection, partial grants, cached access and refresh-token
rotation, private writes, PKCE callback validation, loopback token authorization,
account isolation, Rclone token renewal, Groups helper visibility/batching, and
inventory rollback. These checks cannot prove that an organization's Apps Script
API executable is deployed or that its policy allows ordinary users. Complete the
live deployment acceptance check before production use of My Groups.
