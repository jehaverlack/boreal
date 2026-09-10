# One Google connection for Boreal

Status: implementation candidate, September 2026. The shared connection and My Groups
helper are implemented locally. Live acceptance of a shared Apps Script deployment
is still required. See [migration and setup](GOOGLE-AUTH-MIGRATION.md). The design
below also records future scope reductions and public deployment work.

## Decision

Make Google an account connection shared by independently enabled services. A user
chooses services, signs in once, and then uses Update metadata. Google Cloud project
creation, API enablement, OAuth configuration and Groups helper deployment belong
to the application publisher or organization deployment owner, once per deployment.
They must not be routine tasks for each user.

Use a single organization-managed deployment first for this Workspace rollout.
Distribute its Google app configuration with Boreal or as one organization setup
profile. Later, a Boreal-managed public deployment can provide the same experience
across organizations, after the applicable Google verification and deployment
validation. Keep custom Google projects in Advanced settings.

The current Directory API connector remains an explicit administrative option.
Ordinary My Groups setup must not request a Workspace administrator role.

## End-user experience

1. Open Settings → Google → Connect.
2. Choose Drive inventory, My Groups and/or Persons from Google Sheets. Choose
   migration access explicitly if needed; leave it off by default. Enter the Persons
   Sheet URL when that source is selected.
3. Sign in to Google and approve the permissions for those choices in one flow.
4. Boreal checks the granted permissions and makes small read-only requests to
   verify the selected services. Show an individual result for each service.
5. Select Finish. Thereafter, Update metadata refreshes the enabled, ready sources
   using saved authorization. Remember update selections.

The Google Settings entry shows the account, enabled services, access granted,
connection health, and last successful check. Drive and Groups retain their existing
explorers and green Google navigation with service icons. Service Configure dialogs
edit source options and link to this shared account; they do not have competing
Google sign-in buttons. Persons remains an independent dataset with manual and CSV
use available without any Google connection.

Provide one account-level Reconnect action only when authentication needs repair.
Do not send people through consent for network errors, quotas, hidden members, or
ordinary access-token expiry. An unavailable service must not prevent setup or
updates of the others. Failed updates retain the previous successful inventory;
acknowledging the result returns to the primary Update dialog.

## Groups access

The preferred candidate is a small, versioned Apps Script API executable using
`GroupsApp`, deployed centrally and called with the signed-in user's OAuth token.
It returns a bounded metadata response directly to Boreal. Users must not create
scripts, deploy web apps, export files manually, or grant script-editing access.

Google requires the executable and calling OAuth client to share a standard Google
Cloud project, and the Apps Script API must be enabled there. The deployment owner
configures who may execute it. This is an application deployment task, not an
individual administrator-role assignment. API executables have ownership and scope
constraints that must be tested before rollout. See [Google's execution requirements](https://developers.google.com/apps-script/api/how-tos/execute).

`GroupsApp.getGroups()` returns direct and pending memberships, not inherited parent
memberships. Preserve pending membership explicitly rather than counting it as
confirmed. See [GroupsApp](https://developers.google.com/apps-script/reference/groups/groups-app).

Member visibility is still controlled by Google. `getUsers()` can include banned
users and only returns users with corresponding Google accounts; direct child
groups require a separate call. Return member roles/status and distinguish hidden,
limited, empty and failed results. Group display names, descriptions, aliases and
authoritative total member counts are not all available through this interface;
leave unavailable fields unknown and label counts as returned members. Never present
this as a complete administrative directory. See [Group methods](https://developers.google.com/apps-script/reference/groups/group).

The helper must derive identity from its execution context, accept no caller-supplied
account to impersonate, and avoid logging group/member data. Bound work and payload
size; use resumable batches if necessary. A timeout or quota failure must not replace
an inventory with a truncated result. Execution limits are documented in [Apps Script quotas](https://developers.google.com/apps-script/guides/services/quotas).

Alternatives considered:

| Approach | Decision |
| --- | --- |
| Directory API with per-user Groups Reader role | Keep for administrative deployments; fails the ordinary-user requirement. |
| Cloud Identity transitive membership search | Optional future capability; edition and membership-visibility restrictions make it unsuitable as the universal default. [Google requirements](https://docs.cloud.google.com/identity/docs/how-to/query-memberships). |
| Groups Settings API | Does not provide the required discovery and member-list operations. [API methods](https://developers.google.com/workspace/admin/groups-settings/v1/reference/groups). |
| Per-user Apps Script setup or manual JSON exports | Adds recurring or technical user work; does not meet the desired setup experience. |
| Central directory inventory using administrative credentials | Different access model; requires a server-side authorization design and potentially exposes information beyond the user's group visibility. Not the My Groups default. |

## Permissions and persistence

Request the union of permissions needed for the services selected before sign-in.
Validate the scopes Google actually grants, including partially declined consent.
Do not request every possible future permission merely to avoid future consent.

| Capability | Proposed permission approach |
| --- | --- |
| Account identity | OpenID Connect identity and verified email; key the account by Google's stable subject identifier. |
| Drive inventory | Prefer `drive.metadata.readonly`; verify all inventory and permission-list operations before reducing the current `drive.readonly` grant. |
| My Groups | `https://www.googleapis.com/auth/groups` for the Groups-only helper. Do not request Admin SDK scopes in ordinary mode. |
| Persons Sheet | `spreadsheets.readonly` with a native Sheets reader, so a Drive inventory connection is not a prerequisite. |
| Drive migrations | Explicit `drive` access when selected. Keep existing operation review and destination-permission checks. |

`drive.file` does not cover an inventory of arbitrary existing files. A token with
full Drive access is not made read-only by configuring a remote named `my-drive-ro`.
If migration access is granted, label the account accurately; inventory jobs remain
read-only operations but do not have a separate Google-enforced read-only token.
Retain separately authorized accounts as an advanced option for users who need
that separation. See [Drive scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth).

Use offline authorization, one local credential owner per account, synchronized
token refresh, and private credential storage. Keep tokens out of browser pages,
inventory, exports, logs and subprocess arguments. Track the client ID and granted
scopes with the account. Reconnect replaces authorization atomically without
discarding old inventory. Different Google accounts retain separate credentials
and source assignments; never silently combine them.

External OAuth applications left in Testing receive seven-day refresh tokens for
these scopes. Production deployment must address that before promising durable
connections. Revocation, organizational policy and expired grants can still require
sign-in; no application can guarantee authorization forever. Enabling a previously
unapproved capability may also require additional consent. Routine metadata updates
must not. See [Google's token lifecycle](https://developers.google.com/identity/protocols/oauth2).

## Deployment ownership

For the initial Workspace deployment, its owner prepares one project, Desktop OAuth
client and Groups executable, enables the APIs, configures the correct OAuth
audience, and obtains application approval if organization policy requires it.
The organization profile contains application identifiers and supported capabilities,
never a user's refresh token or administrative credentials.

For a public Boreal-managed deployment, the publisher owns these steps and Google's
app verification. Broad Drive scopes are restricted; verification requirements and
any applicable assessment must be established for the final data flow. Keep Drive
and Sheet data flowing directly between the local app and Google APIs; keep the
Groups helper scoped to Groups only. A public rollout is not achieved merely by
embedding a client ID. See [verification requirements](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification).

## Implementation sequence and release checks

1. **Prove the Groups path first.** Use the same project and Desktop OAuth client
   for a deployed executable and a local PKCE sign-in. Test a normal Workspace
   member and a second non-owner user without Admin SDK roles, editor access or
   per-user script setup. Verify direct groups, visible/hidden members, pending and
   banned users, expiry/refresh, quotas and isolation between users. Test an external
   account before claiming support across organizations. The docs support the
   candidate; this live acceptance test is still required.
2. **Introduce shared Google accounts.** Extract OAuth and refresh from
   `src/google/groups.rs` into a provider-level account component. Replace direct
   cached-token reads in `src/rclone/identity.rs`; implement independent Sheet
   access. Store granted capabilities separately from enabled datasets.
3. **Integrate Rclone without another sign-in.** Keep existing remotes compatible,
   but make managed remotes adapters for the selected Google account. Prove token
   handoff and refresh over long-running jobs and concurrent updates using the
   shipped Rclone version. Do not blindly copy refresh tokens into independent
   stores with competing refresh owners. Preserve additional user-managed remotes.
4. **Replace onboarding.** One Google connection dialog and one consent flow;
   per-service live checks and source options; project instructions confined to
   deployment/Advanced setup. Preserve navigation, independent service toggles,
   and modal acknowledgement behavior.
5. **Migrate existing installations.** Offer one combined authorization for their
   selected services. Preserve cached data, tags, account boundaries and remote
   names. Retain old credentials until the replacement works. Map Groups identities
   deliberately because Apps Script emails differ from Directory IDs; do not lose
   tags or invent authoritative IDs/counts.
6. **Accept the end-user flow.** A fresh ordinary user connects once, updates Drive,
   Groups and the optional Sheet, restarts Boreal, then updates again after access
   token expiry without opening a browser. Test declined scopes, disabled services,
   switching accounts, revoked access and failed member queries. Verify unrelated
   services still update and failures cannot erase a successful inventory.

Do not mark the new Groups path production-ready until step 1 passes. If it fails
under the intended deployment policy, report that specific limitation before
reworking onboarding around another unproven connector.
