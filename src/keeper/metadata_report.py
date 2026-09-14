"""Run inside Keeper Commander's Python environment; emit metadata only.

Never export a record, call get_record_description, or serialize a cache entry.
Only build the explicit output schema below. Commander retains all secret values.
"""
import argparse
import contextlib
import json
import logging
import os
import sys


def text(value):
    return value if isinstance(value, str) else ""


def number(value):
    return value if type(value) is int and 0 <= value <= 9223372036854775807 else 0


def object_data(value):
    if isinstance(value, dict):
        return value
    result = json.loads(value or "{}")
    if not isinstance(result, dict):
        raise ValueError("Invalid metadata")
    return result


def snapshot(params, sharing_rows):
    folders = {"": dict(folder_uid="", parent_uid="", name="My Vault",
                         folder_type="Vault", folder_path="/", access=[])}
    for uid, folder in params.folder_cache.items():
        if not uid:
            continue
        folders[uid] = dict(folder_uid=uid, parent_uid=folder.parent_uid or "",
                            name=text(folder.name), folder_type={
                                "shared_folder": "Shared Folder",
                                "shared_folder_folder": "Folder in Shared Folder",
                                "user_folder": "Folder",
                            }.get(folder.type, "Folder"), folder_path="", access=[])
    # Newer Commander releases also maintain nested shared-folder caches.
    for uid, folder in (getattr(params, "nested_share_folders", {}) or {}).items():
        if uid not in folders:
            folders[uid] = dict(folder_uid=uid, parent_uid=text(folder.get("parent_uid")),
                                name=text(folder.get("name")), folder_type="Nested Share Folder",
                                folder_path="", access=[])
    for row in sharing_rows:
        uid = row["Folder UID"]
        if uid not in folders:
            # A reported shared folder must remain browsable even if not mounted.
            folders[uid] = dict(folder_uid=uid, parent_uid="", name=text(row["Folder Name"]),
                                folder_type=text(row.get("Type")), folder_path="", access=[])
        folders[uid]["folder_type"] = text(row.get("Type")) or "Shared Folder"
        target = text(row.get("Shared To"))
        if target:
            folders[uid]["access"].append(dict(shared_to=target,
                permissions=text(row.get("Permissions")), target_kind=(
                    "team-user" if target.startswith("(Team User)") else
                    "team" if target.startswith("(Team)") else "user")))
    for uid, folder in folders.items():
        if folder["parent_uid"] not in folders:
            raise ValueError("Missing parent folder")
        parts, seen, current = [], set(), uid
        while current:
            if current in seen:
                raise ValueError("Cyclic folder hierarchy")
            seen.add(current)
            parts.append(folders[current]["name"])
            current = folders[current]["parent_uid"]
        folder["folder_path"] = "/" + "/".join(reversed(parts))

    caches = dict(params.record_cache or {})
    for uid, entry in (getattr(params, "nested_share_records", {}) or {}).items():
        # Nested metadata often lacks decrypted data; retain the classic cache copy.
        caches[uid] = dict(caches.get(uid, {}), **entry)
    nested_data = getattr(params, "nested_share_record_data", {}) or {}
    for uid, entry in nested_data.items():
        if "data_unencrypted" not in caches.get(uid, {}) and "data_json" in entry:
            caches[uid] = dict({"version": 3}, **caches.get(uid, {}))
            caches[uid]["data_unencrypted"] = entry["data_json"]
    records = {}
    for uid, cached in caches.items():
        if "data_unencrypted" not in cached:
            raise ValueError("Record metadata unavailable")
        data = object_data(cached["data_unencrypted"])
        version = number(cached.get("version"))
        # Inspect only structure for attachment counts, never field values in output.
        extra = object_data(cached.get("extra_unencrypted"))
        if version == 2:
            files = extra.get("files") or []
            attachment_count = len(files)
            attachment_bytes = sum(number(file.get("size")) for file in files)
        else:
            references = {uid for field in (data.get("fields", []) + data.get("custom", []))
                          if field.get("type") == "fileRef"
                          for uid in (field.get("value") or []) if isinstance(uid, str)}
            attachment_count = len(references)
            attachment_bytes = sum(number(object_data(caches.get(uid, {}).get("data_unencrypted")).get("size"))
                                   for uid in references)
        records[uid] = dict(record_uid=uid, title=text(data.get("title")),
            record_type=text(data.get("type")) or {2: "login", 4: "file", 5: "application"}.get(version, "record"),
            modified_ms=number(cached.get("client_modified_time")),
            version=version, attachment_count=attachment_count,
            size_bytes=number(data.get("size")) if version == 4 else attachment_bytes)
    memberships = set()
    for mapping in (params.subfolder_record_cache or {},
                    getattr(params, "nested_share_folder_records", {}) or {}):
        for folder_uid, uids in mapping.items():
            if folder_uid not in folders:
                raise ValueError("Missing record folder")
            for uid in uids:
                if uid not in records:
                    raise ValueError("Missing record metadata")
                memberships.add((folder_uid, uid))
    # File records attached to another record are not standalone root entries.
    located = {uid for _, uid in memberships}
    for uid, record in records.items():
        if uid not in located and record["version"] != 4:
            memberships.add(("", uid))
    visible_records = {uid for _, uid in memberships}
    return dict(schema_version=1, folders=list(folders.values()),
                records=[record for uid, record in records.items() if uid in visible_records],
                memberships=[dict(folder_uid=f, record_uid=r) for f, r in sorted(memberships)])


def collect_report(config):
    from keepercommander import api
    from keepercommander.__main__ import get_params_from_config
    from keepercommander.commands.utils import LoginStatusCommand
    from keepercommander.commands.register import ShareReportCommand

    params = get_params_from_config(config)
    # This command uses Commander's noninteractive login UI and cancels any
    # new MFA/SSO prompt. Check the actual session, not the printed status.
    LoginStatusCommand().execute(params)
    if not params.session_token:
        raise PermissionError("Session required")
    api.sync_down(params)
    sharing = ShareReportCommand.sf_report(params, fmt="json")
    return snapshot(params, json.loads(sharing))


def main():
    report = None
    exit_code = 0
    # Suppress all Commander banners, prompts, diagnostics and tracebacks at source.
    # Only the constructed report may cross the subprocess boundary into BOREAL.
    with open(os.devnull, "w") as sink, contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
        logging.disable(logging.CRITICAL)
        try:
            parser = argparse.ArgumentParser()
            parser.add_argument("--config", required=True)
            args = parser.parse_args()
            report = collect_report(args.config)
        except PermissionError:
            exit_code = 2
        except BaseException:
            exit_code = 1
    if exit_code != 0 or report is None:
        sys.stderr.write("Keeper metadata report failed; verify the session and Commander compatibility.\n")
        return exit_code or 1
    sys.stdout.write(json.dumps(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
