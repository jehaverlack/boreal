# Boreal My Groups deployment

Deploy this helper once per Boreal Google Cloud project. Users execute it using
**their own Google authorization**; they do not need a Workspace Groups Reader role.
This replaces the Directory API path for ordinary My Groups imports.

1. In the Boreal Google Cloud project, enable **Apps Script API**. Enable **Google
   Drive API** as well when using Drive or a private Persons Sheet.
2. Create a Desktop OAuth client. For an internal Workspace deployment, configure
   the appropriate internal OAuth audience. External production applications must
   address Google's verification requirements and leave Testing mode; testing
   refresh tokens for these permissions expire after seven days.
3. Create a standalone Apps Script project. Add `Code.gs` from this directory.
   Enable **Show appsscript.json manifest file** in Project Settings and replace
   the manifest with the supplied `appsscript.json`.
4. In Apps Script Project Settings, select **Change project** and enter the
   **Google Cloud project number** used by Boreal's Desktop client. A script's
   default automatically generated Cloud project is not sufficient.
5. Choose **Deploy → New deployment → API executable**. Select an access audience
   that includes the intended users. Deploy a version and copy its deployment ID.
   Do not deploy as a web app running as the owner.
6. In Boreal, open **Settings → Google connection**. Import the Desktop client
   JSON through the project credentials link, enter the deployment ID, and save.
7. Download the reusable Google setup profile on that page. It contains Desktop
   application configuration and helper settings, never user refresh tokens.
   Other installations import that file using the same project credentials upload.
   Users choose services and click **Connect Google and check access** once.
8. Verify with an ordinary account that does not own the script or have Directory
   API administrative privileges. The access check must return that user's groups.
   Afterward, use **Update metadata** for imports.

Do not enable Groups Settings API or Admin SDK for this mode. Admin SDK is an
explicit advanced alternative in Boreal, for accounts already granted Directory
read privileges. Organization app-access policy may still require approval of the
application; the helper does not bypass that policy.

The helper returns direct/pending groups, visible direct user members, user roles,
and child groups. Child group roles are unknown; their members are not expanded.
Users without corresponding Google accounts may be omitted by GroupsApp. Names,
descriptions, aliases and authoritative member counts are not supplied by that
interface. Boreal displays unknown counts and marks unavailable member lists.

Groups are fetched in batches of ten. A changed group list aborts a paged report;
quota or unexpected query failures preserve the previous inventory. Member-visibility
errors mark that list unavailable. No vault data, Google messages, credentials or
provider error text enter the report or helper logs.

This helper has local mocked coverage. A live shared deployment and ordinary-user
acceptance test are required before production rollout. The desktop application
cannot create this deployment from the user's existing Directory authorization.

References: [Apps Script execution requirements](https://developers.google.com/apps-script/api/how-tos/execute),
[GroupsApp](https://developers.google.com/apps-script/reference/groups/groups-app),
[Group methods](https://developers.google.com/apps-script/reference/groups/group).
