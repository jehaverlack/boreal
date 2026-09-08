# Security Policy

## Supported versions

BOREAL is currently maintained as a single release line. Security fixes are
provided for the latest release in the current minor series. Users should
upgrade to the newest available release before reporting a problem that may
already have been corrected.

| Version | Supported |
| --- | --- |
| 1.1.x | :white_check_mark: |
| < 1.1 | :x: |

The current release is listed on the
[BOREAL Update page](https://github.com/jehaverlack/boreal/releases/latest)
and in the repository's `changelog.json` file.

## Reporting a vulnerability

Please report suspected security vulnerabilities privately through
[GitHub Private Vulnerability Reporting](https://github.com/jehaverlack/boreal/security/advisories/new).

Do **not** disclose a suspected vulnerability in a public GitHub issue,
discussion, pull request, log excerpt, or screenshot. Public issues remain the
right place for ordinary bugs and feature requests that do not contain
sensitive information.

Include as much of the following information as is safe and relevant:

- The affected BOREAL version and operating system.
- A concise description of the vulnerability and its potential impact.
- Reproduction steps or a minimal proof of concept.
- Whether exploitation requires local access, a browser action, or configured
  Google Drive, GitHub, Keeper, or Rclone credentials.
- Suggested mitigations, if known.

Never include real OAuth credentials, access or refresh tokens, Keeper SSO
URLs or tokens, passwords, private repository data, personal information, or
an unredacted BOREAL configuration/database. Replace sensitive values with
clearly marked placeholders.

If the private-reporting link is unavailable, do not post technical details
publicly. Contact the repository maintainer through their GitHub profile and
request a private reporting channel.

## What to expect

- Receipt should be acknowledged within three business days.
- An initial assessment should be provided within seven business days when
  enough information is available to reproduce or evaluate the report.
- Confirmed vulnerabilities will be handled privately while a mitigation,
  patch, and release plan are prepared.
- The reporter may be asked to verify a proposed fix and will be credited in
  the advisory when desired and appropriate.
- If a report is declined, the response will explain why it is considered out
  of scope, not reproducible, or not a security vulnerability.

Timelines may vary with severity and complexity. Please allow time for a fix
before public disclosure; a coordinated disclosure date will be discussed
with the reporter.

## Security scope

Security reports are especially useful when they concern:

- Exposure of Google OAuth, Rclone, GitHub, or Keeper credentials.
- Access to BOREAL's localhost WebUI from an unintended origin or user.
- Unsafe command construction, path handling, file permissions, or archive
  extraction.
- Unauthorized modification, deletion, migration, or disclosure of indexed or
  remote content.
- Cross-site scripting, request forgery, or injection through imported or
  remotely supplied metadata.
- Distribution, update, or dependency-integrity weaknesses.

Reports about an unsupported BOREAL release may be closed after confirming
whether the issue is present in the latest supported release. Vulnerabilities
in Rclone, Keeper Commander, GitHub, Google services, browsers, or operating
systems should also be reported to the corresponding upstream project when
the issue is not caused by BOREAL.

## BOREAL's trust boundary

BOREAL is designed as a single-user local desktop application. Its WebUI binds
to the loopback interface, and its configuration, credentials, logs, and
SQLite inventory are stored under the current user's BOREAL directory. This
model assumes that the local operating-system account and browser session are
trusted; it does not make vulnerabilities that cross those boundaries out of
scope.
