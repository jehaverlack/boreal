# Developer and Release Workflow

The `main` branch represents the most recent released version. Annotated
`vX.Y.Z` tags preserve each release. Development for the next release occurs on
a matching `bX.Y.Z` branch. The different prefixes prevent branch and tag name
collisions.

## 1. Develop on the version branch

Commit and test changes on the active version branch:

```bash
git status --short
cargo check
cargo test
```

## 2. Finalize metadata and changelog

Update the active changelog entry and propagate current metadata into managed
files:

```bash
./tools/changelog-update.sh
```

Review and edit the active release summary and notes in `changelog.json` when
needed, then regenerate the Markdown changelog:

```bash
./tools/genmd-changelog.sh
```

Commit the final documentation and metadata changes before building release
artifacts.

## 3. Build release binaries

On the macOS build host:

```bash
./tools/build-release.sh
```

Copy the versioned macOS binaries from `build/` to the Linux build host's
`build/` directory.

On the Linux build host:

```bash
./tools/build-release.sh
```

The Linux host produces the enabled Linux and Windows targets. The collected
`build/` directory should then contain every target enabled in `metadata.json`.

## 4. Stage the release

Validate the complete artifact set, copy the versioned binaries into `dist/`,
and generate `dist/SHA256SUMS`:

```bash
./tools/stage-release.sh
```

Smoke-test the staged binaries on their target operating systems before
publishing the release.

## 5. Merge, tag, and push the release

The version branch and working tree must be clean:

```bash
./tools/main-merge.sh
git push origin main
git push origin --tags
```

`main-merge.sh` merges the `bX.Y.Z` version branch into `main`, creates the
annotated `vX.Y.Z` version tag, and removes the local version branch. Legacy
`vX.Y.Z` release branches are also accepted so an in-progress release is not
broken by the naming transition.

## 6. Publish the GitHub release

Create a GitHub release from the new version tag. Upload:

- Every versioned binary from `dist/`.
- `dist/SHA256SUMS`.
- Release notes based on the current changelog entry.

Verify that every README download link works after the assets finish uploading.

### Verify update discovery

BOREAL checks `https://raw.githubusercontent.com/jehaverlack/boreal/main/changelog.json`
at startup, every six hours, and when **Upgrade → Check for updates** is clicked.
It compares the highest valid numeric release version with the running binary's
compiled Cargo version. Editing `metadata.json` alone does not publish an update.

Before announcing a release, verify that the public `main` changelog includes
the new version, its `vX.Y.Z` tag exists, and the tagged `dist/` binaries are
accessible. The in-app download button uses the binary in the tag's `dist/`
directory; the **View release** button opens the GitHub release page. Test the
check from an older build that uses the current feed, and confirm both links work.
Keep future development changelog entries on the version branch until release.

Older builds, including v1.1.1, use the legacy `jehaverlack/uaf-boreal` feed.
They need a manual upgrade unless that repository's `main/changelog.json` is
also maintained with the published release entry. A fix in a new binary cannot
change the feed URL compiled into an already installed binary.

## 7. Start the next version

From the clean, current `main` branch:

```bash
./tools/new-version.sh
```

This creates the next version branch and updates version metadata. README
release filenames and links are updated by `tools/metadata-update.sh`.
