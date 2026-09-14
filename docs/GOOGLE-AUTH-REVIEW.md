# Google authentication review — 2026-09-11

## Recommendation

Restore the Rclone authentication behavior from `011db32` (Deep Dive UX), retaining
subsequent targeted reconnect fixes and independent UI fixes. Defer Google Groups
from the stable 1.2 release. Abandon the Apps Script deployment as the default
end-user setup. Do not replace it immediately with another production auth rewrite.

Keep the Groups viewer/database work available for a separate, bounded Cloud
Identity experiment. Only bring Groups back into the normal setup after an ordinary
Workspace account can discover its groups, read permitted members, and refresh
metadata after restart without additional setup or changes to Drive authorization.

This is a review and proposed recovery scope. No application code, live settings,
credentials, or Git history were changed as part of this review. This report is the
only new repository change for the review.

## Compared revisions

| Revision | Change | Assessment |
| --- | --- | --- |
| `011db32` Deep Dive UX | Modular Settings and existing explorers; optional Directory-based Groups already present | Reference for working core behavior, not proof that ordinary-user Groups worked |
| `8b025fa` Google Services | Rclone browser authorization flags and sanitized diagnostics | Retain: fixes noninteractive create/reconnect rather than changing auth ownership |
| `9d331a3` Failed GGroup Attempt | Directory error classification, readiness checks, Settings form fixes | Retain independent UI fixes; do not equate clearer 403 messages with working ordinary-user Groups |
| `560cb1a` Major Refactor | Shared account store, token renewal bridge, removal of normal Rclone reconnect path, Apps Script Groups | Main architectural regression boundary |
| Current working changes | Terminology cleanup and repeated account/project setup UI revisions | Keep useful terminology selectively; these changes still depend on the refactored auth model |

The committed delta from Deep Dive UX to Major Refactor is 36 files, 2,703 added
lines and 602 removed lines. Most changes concern authentication, setup and Groups.
Cargo.toml, Cargo.lock, database migrations, src/main.rs, tmpl/html/base.html and the
Keeper/GitHub/local-files source directories have no diff from the reference in the
current working tree. Shared routes and app orchestration do differ, so this does
not certify every non-Google UI path as unaffected.

## Findings

### 1. High: existing Rclone accounts are no longer represented or repairable through their original UI

`src/web/routes.rs:3753` now redirects `/remotes/add` to `/google`. The original
Rclone configure/reconnect functions in `src/rclone/remotes.rs` are compiled only
under `cfg(test)` (configure begins at line 138). The new account page reads
`auth::email`, which reads only `google-account.json`.

Trigger: upgrade a working installation that has Rclone tokens but no shared account
file, then inspect its sign-in state or reconnect a managed remote. The new UI says
Google is not signed in and pushes the user into a new authorization workflow,
instead of showing that the existing Drive authorization is still configured.

The local read-only inspection found both managed remotes, their expected Drive
scopes, nonempty tokens and matching current client IDs. The new account file is
absent. This confirms the misleading state for this installation; it does not prove
that Google's servers will currently accept either legacy token.

### 2. High: unfinished Groups setup blocks otherwise independent Google sign-in

`src/web/google_connection.rs:59` routes selected Groups with a missing deployment
to project setup. `connect` invokes this before OAuth at line 241. The committed
Major Refactor returned an error for this condition; the working UI now redirects,
but the dependency remains.

Trigger: Drive and Groups are selected, a Desktop client is configured, and there
is no Groups deployment ID. The user cannot reconnect Drive or its private Persons
Sheet through the normal shared sign-in action without completing Groups setup or
turning Groups off. A less important optional source controls recovery of core
services. This is the reported setup loop expressed in code.

### 3. High: an incomplete new grant can displace working Drive authorization

`src/google/auth.rs:428` saves the newly authorized identity/scopes before the
service checks in `src/web/google_connection.rs:252`. There is no requirement that
all selected Drive permissions were actually granted before installing that account
as the active one. `src/app.rs:485`, `src/google/bridge.rs:37`, and
`src/rclone/identity.rs:309` switch to the shared store based on file existence.

Trigger: complete Google consent but grant Groups/identity without Drive, or later
replace an existing shared grant with insufficient permissions. The account file
becomes authoritative immediately. Drive and private Sheets report missing scope
even if old Rclone authorizations remain usable. Service checks report the problem
afterward; they do not preserve the earlier active auth selection.

This is a code-confirmed failure path, not a live reproduction against Google.
Recovery should use explicit account/source bindings or a staged, validated
transition. Silently falling back to a different Google account would introduce a
different correctness problem and is not the proposed fix.

### 4. High for affected migrations: separately authorized source and destination accounts were collapsed

`src/google/bridge.rs:73` builds both `my-drive-ro` and `my-drive-rw` with the same
local token capability and account. The legacy UI in Deep Dive UX authorized each
remote independently, including a different account for migration destinations.

Once shared auth is active, a migration formerly reading through account A and
writing through account B uses the single shared account for both. Access failures
follow if that account cannot see both sides. The temporary config also replaces
legacy backend options with a minimal pair of Drive remotes; settings not supplied
as command-line options no longer carry over.

The current user's two account identities were not queried, so this finding does
not assert that their particular migration uses different accounts.

### 5. Medium: configured permissions are treated as service readiness after access checks fail

`src/web/google_connection.rs:266` collects service failures as strings and returns
`Ok` at line 315. `src/google/auth.rs:132` determines readiness using stored client,
refresh-token presence and scopes, not the last successful API check. Groups uses
that local readiness to enable update actions.

Trigger: consent succeeds, but Apps Script is disabled, misdeployed or inaccessible.
The check displays an explanatory message but leaves the service eligible for
metadata updates. The next update fails again. A saved grant and a verified data
source need distinct states, with a durable recovery action for the failed source.

### 6. Medium: replacing a Desktop client can retain an incompatible Groups deployment

`src/google/client.rs:129` replaces the Desktop JSON. It changes Groups setup only
when the imported file contains `boreal_google`. Importing an ordinary Desktop JSON
from another project leaves the old deployment ID in place. Apps Script requires
the calling OAuth client and script to use the same Cloud project.

Trigger: a user follows Replace Google setup with another project's Desktop JSON.
Reconnecting can fix the OAuth client mismatch while leaving Groups unable to run.
Project/deployment compatibility is not invalidated or re-established in that path.

## Proposed recovery scope

1. Restore normal managed Rclone configure/reconnect and the original Drive account
   selection. Retain the `--auto-confirm`/secret-handling fixes from Google Services.
2. Remove the shared-token bridge from the stable Drive execution paths. Preserve
   existing remote configuration and data. A machine that already switched to shared
   auth may need explicit Rclone reconnection; do not copy or delete tokens blindly.
3. Defer Groups indexing and its setup prompts from the stable UI. Preserve cached
   Groups data and the viewer/schema work for possible reuse. Disabling Groups alone
   is insufficient: the shared account file still switches Drive auth behavior.
4. Retain the corrected Storage remotes terminology, Settings modal/form fixes,
   Persons independence for local entries/CSV, and existing explorer work. A private
   Persons Sheet must continue to use authorized Google access.
5. Validate a fresh install and an upgraded install: Drive inventory, one Shared
   Drive, shared-with-me, a download, private Sheet refresh, reconnect, restart/token
   renewal, and a reviewed migration using the intended source/destination accounts.
   Groups being unavailable must not prevent any of these operations.

Use the reference as a behavioral baseline and restore the affected paths
selectively. A blanket hard reset would discard useful later fixes and the current
uncommitted work. No database schema rollback is indicated by this comparison.

## Whether to pursue Groups later

Cloud Identity is a reasonable isolated experiment, not a verified replacement.
Google documents `searchDirectGroups` for direct memberships, including the fact
that groups whose memberships the caller cannot view are silently filtered out.
It documents non-admin end-user OAuth authentication, but also domain prerequisites
including Groups for Business and its sharing configuration. These are not a
promise of universal access after a single OAuth click.

A small prototype should use separate experimental credentials/configuration and
leave working Drive/Rclone credentials untouched. Acceptance requires an ordinary
user, expected visible group coverage, pagination and visible members, clear handling
of restricted results, and a second metadata update after token refresh. If the
organization's restrictions or setup requirements defeat the product's simple-user
setup goal, leave Groups unsupported. Do not add another fallback wizard.

The earlier strategy considered Cloud Identity transitive search. Direct group
search is a separate method; that distinction was missed before the Apps Script
implementation was expanded. User-account validation should have preceded the
shared authentication rewrite.

Sources: [direct group search](https://docs.cloud.google.com/identity/docs/reference/rest/v1/groups.memberships/searchDirectGroups),
[Cloud Identity setup and end-user authentication](https://docs.cloud.google.com/identity/docs/how-to/setup),
[Apps Script deployment requirements](https://developers.google.com/apps-script/api/how-tos/execute).

## Validation performed and limits

- Reviewed the commit history, reference-to-working-tree diffs, production call
  paths and existing test coverage.
- Read only allowlisted local configuration facts; no credential values were
  printed, files changed, or Google account authorization attempted.
- `node tools/test-web-controls.cjs` and
  `node tools/test-google-groups-helper.cjs` passed. `git diff --check` passed.
- A fresh Rust test run could not complete: the offline cache lacked
  `serde_spanned 1.1.1`; an authorized retry then failed parsing the cached
  `tinyvec_macros 0.1.1` package because it had no targets. This is a dependency-cache
  failure, not an observed application test failure. Earlier passing runs are not
  presented as new validation of this review.
- No live Drive/Groups/Sheet queries or migrations were performed. Findings are
  static code paths and local configuration observations, with conditions stated
  above. Google Groups remains unverified for this account.
