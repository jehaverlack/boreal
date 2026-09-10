# Service setup and configuration UX review

September 2026. Scope: first launch, Settings, Google setup, account connections,
service setup helpers, dashboard, update entry points, and navigation consistency.

The replacement for the separate Google authorizations is documented in
[One Google connection for Boreal](GOOGLE-CONNECTION-STRATEGY.md). It separates
one-time deployment setup from ordinary user sign-in and metadata updates. The
findings below describe the earlier implementation. See [Google auth migration](GOOGLE-AUTH-MIGRATION.md) for the current shared connection and its validation limits.

## Findings and changes

| Surface | Problem | Result |
| --- | --- | --- |
| Settings | A long form mixed enablement, authentication, provider-specific fields and unrelated test actions. Saving one service could change another. | One service table with enabled state, setup status, next step and Configure action. Seven separately scoped forms in scrollable dialogs; each has Save changes and Cancel. |
| Default Drive connections | Existing remote names removed the Add option even when the configuration conflicted. Conflict badges provided no recovery path. | Keep the default names. Explain conflicts without credential values and offer explicit Repair / reconnect. Missing defaults can be added again. |
| Additional connections | The Add dialog only supported two fixed names. | Add connection also leads to the managed Rclone connection manager for additional named Drive accounts and other storage providers. Refresh reads the current configuration. Additional Drive connections are not automatically included in inventory. |
| Google setup | Separate credential instructions and a newer API guide competed for entry points. | Settings hosts the shared Google setup guide, credential upload/replace control and credential error state. The detailed credential walkthrough returns to Settings and covers both Drive and Admin SDK. |
| Google navigation | Renaming the menu dropped its green class and several item icons. | Preserve green Google navigation and official G; retain the Drive divider/icon and individual submenu icons. Regression tests cover enabled-service combinations. |
| Dashboard | Incomplete Google onboarding replaced the whole dashboard, hiding other configured services. | Dashboard continues to show services. Setup links lead to Settings. |
| Persons | People and the optional Google Sheet appeared to be one Google-dependent integration. | Persons is always available. Manual entries and CSV need no Google service. Optional Sheet configuration and testing live in the Persons dialog. |
| Keeper | Setup instructions and test actions sat in the main settings form; one help entry incorrectly recommended standalone executables. | Keeper configuration and testing live in its service dialog. The installation helper remains available; help consistently specifies the Python environment required by the metadata helper. |
| GitHub | Account-specific tokens and general settings were mixed together. | The GitHub dialog owns its connection list and token assistant. Token forms remain separate from settings forms. |
| S3 | Users had to type an already configured remote name. | Choose from configured S3 connections; creation is available through Connections. |
| Errors and completion | Setup actions could lead to a bare conflict response or an unexpected dashboard redirect. | Connection-start failures return an actionable Connections page. Saving a service returns to Settings. Credential upload returns to Google setup. Update retains its acknowledge-and-return behavior. |

## Consistency contract for future services

- Start at Settings. Show availability, configuration state and the next step separately.
- Use one Configure dialog per service. Keep credentials out of summaries and inventory.
- Save only that service's fields. Cancel discards unsaved edits. A missing checkbox in
  one service's form must never disable another service.
- Use progressive disclosure for installation instructions and advanced provider options.
- Route setup failures back to an actionable surface. Distinguish stored configuration
  from verified live access; installing a tool is not proof of authorization.
- Preserve established service colors, icons, explorer filters, sort state and column widths.
- Use Update for imports; configuring or enabling a service does not silently start an import.

## Provider boundaries

Google authorization still happens on Google's page. Keeper sign-in remains in Commander
so sensitive vault values stay outside Boreal. Additional storage connections use the
managed Rclone WebGUI; Boreal does not yet provide a native configuration dialog for each
Rclone backend. Drive inventory and migrations continue to use the two default remote
names. These boundaries are described in the corresponding dialogs.

The existing copy-and-review migration and metadata-only Keeper behavior are unchanged
by this setup refactor. No account credentials were reauthorized or live data transferred
while validating this change.

## Verification

Rust tests cover per-service save isolation, rejected unknown services, dialog rendering,
conflict redaction, named reconnect commands and Google navigation details. Existing UI
control checks cover update acknowledgement, tag behavior and shutdown messaging.
Chromium checks exercise independent form ownership, modal opening, Cancel/reset,
service deep links, Google styling and submenu behavior. An isolated real Rclone config
check verifies that repair changes the requested remote, clears its stale token, and
preserves unrelated connections.

Rclone command behavior was checked against its official
[config update](https://rclone.org/commands/rclone_config_update/),
[reconnect](https://rclone.org/commands/rclone_config_reconnect/) and
[WebGUI](https://rclone.org/gui/) documentation.

## Reconnect terminal-input regression

Rclone 1.75 prompts for console input during `config reconnect`, even when Boreal
starts it as a background process with closed stdin. The resulting `Failed to read
line: EOF` occurs before the browser opens. Managed create/reconnect calls now use
`--auto-confirm` for Rclone's configuration questions; Google consent still happens
in the browser. Known failures are translated to fixed, actionable messages without
copying credential-bearing subprocess output.

Run `python3 tools/test-rclone-auth.py /path/to/rclone` on Linux to verify both Drive
scopes against a mock loopback OAuth server. This reproduces EOF without the flag,
then exercises create, missing-token reconnect, and existing-token reconnect with
it. The test uses only synthetic credentials and a temporary config, stubs browser
launch, and verifies that the additional connection is preserved. Rclone's local
callback port 53682 must be available.

## Google setup modal and Groups connection recovery

Settings dialogs must remain siblings. A missing closing tag in the S3 partial
previously nested the Google guide inside the hidden S3 dialog, displaying only a
backdrop. Validate rendered Settings after template edits:

```sh
BOREAL_UI_FIXTURE_DIR=/tmp/boreal-ui cargo test settings_dialogs_render_independent_forms_and_connections_offer_repair
python3 tools/test-settings-layout.py /tmp/boreal-ui/settings.html
```

Also exercise the Settings guide button, Drive's Prepare your Google project link,
the guide-to-upload transition, the direct setup hash link, and backdrop cleanup.

Groups maintains authorization separately from Drive. Replacing the Desktop OAuth
Client ID invalidates the saved Groups authorization for future imports. Settings,
the Groups viewer and Update now report that local mismatch and offer a dedicated
Groups reconnect action. The account identity and cached group inventory remain
available for browsing while reconnecting. Readiness does not imply that a live
Google API request has succeeded.

## Directory API access denials

Do not map every HTTP 403 to API enablement. Groups responses now distinguish
`SERVICE_DISABLED`/`accessNotConfigured`, missing OAuth scopes, Workspace resource
authorization, app policy restrictions and quota errors. Only known categories are
shown; raw provider error messages and account details stay out of the UI and logs.
The failing operation is identified as `groups.list` or `members.list`.

For a `forbidden` / resource-authorization response, verify the signed-in user's
Workspace Admin API Groups → Read privilege or Groups Reader role. Cloud project
administration and group ownership are separate from Directory API authorization.
See Google's [administrator privilege definitions](https://knowledge.workspace.google.com/admin/users/administrator-privilege-definitions).
