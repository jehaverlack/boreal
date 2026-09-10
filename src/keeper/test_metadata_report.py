"""Synthetic vault tests: no credentials, network or real Keeper configuration."""
import contextlib
import io
import json
import sys
import types
import unittest
from unittest.mock import patch

import metadata_report as report


class MetadataReportTests(unittest.TestCase):
    def params(self):
        return types.SimpleNamespace(
            folder_cache={
                "personal": types.SimpleNamespace(parent_uid="", name="Personal", type="user_folder"),
                "shared": types.SimpleNamespace(parent_uid="", name="Operations", type="shared_folder"),
                "child": types.SimpleNamespace(parent_uid="shared", name="Nested", type="shared_folder_folder"),
            },
            record_cache={
                "record": dict(version=3, client_modified_time=1700000000000,
                    record_key_unencrypted="SECRET-KEY", data_unencrypted=json.dumps(dict(
                        title="Service account", type="login", notes="SECRET-NOTES",
                        fields=[dict(type="password", value=["SECRET-PASSWORD"]),
                                dict(type="url", value=["https://SECRET-URL"]),
                                dict(type="oneTimeCode", value=["SECRET-TOTP"]),
                                dict(type="fileRef", value=["file"])],
                        custom=[dict(label="SECRET-LABEL", value=["SECRET-CUSTOM"])]))),
                "legacy": dict(version=2, data_unencrypted=json.dumps(dict(title="Legacy", secret1="SECRET-LOGIN", secret2="SECRET-PASSWORD")),
                    extra_unencrypted=json.dumps(dict(files=[dict(id="file", title="SECRET-FILENAME", key="SECRET-FILEKEY")]))),
                "file": dict(version=4, data_unencrypted=json.dumps(dict(title="Attachment", size=321, key="SECRET-KEY"))),
            },
            subfolder_record_cache={"": {"legacy"}, "personal": {"record"}, "child": {"record"}},
        )

    def test_only_allowlisted_metadata_crosses_boundary(self):
        result = report.snapshot(self.params(), [{"Folder UID": "shared", "Folder Name": "Operations", "Type": "Shared Folder", "Shared To": "person@example.test", "Permissions": "Can Manage Users"}])
        output = json.dumps(result)
        self.assertNotIn("SECRET", output)
        self.assertEqual(set(result), {"schema_version", "folders", "records", "memberships"})
        record = next(r for r in result["records"] if r["record_uid"] == "record")
        self.assertEqual(set(record), {"record_uid", "title", "record_type", "modified_ms", "version", "attachment_count", "size_bytes"})
        self.assertEqual(record["modified_ms"], 1700000000000)
        self.assertEqual(record["attachment_count"], 1)
        self.assertEqual(record["size_bytes"], 321)
        self.assertNotIn("file", {r["record_uid"] for r in result["records"]})
        self.assertEqual(next(f for f in result["folders"] if f["folder_uid"] == "child")["folder_path"], "/Operations/Nested")
        self.assertEqual(sum(m["record_uid"] == "record" for m in result["memberships"]), 2)
        self.assertIn(dict(folder_uid="", record_uid="legacy"), result["memberships"])

    def test_nested_share_metadata_preserves_decrypted_classic_data(self):
        params = self.params()
        params.nested_share_records = {"record": {"version": 3, "client_modified_time": 1700000000123},
                                       "nested_record": {"version": 3}}
        params.nested_share_record_data = {"nested_record": {"data_json": {"title": "Nested record", "type": "login", "fields": [{"type": "password", "value": ["SECRET-NESTED"]}]}}}
        params.nested_share_folders = {"nested_folder": {"parent_uid": "shared", "name": "Nested share"}}
        params.nested_share_folder_records = {"nested_folder": {"nested_record"}}
        result = report.snapshot(params, [])
        self.assertNotIn("SECRET", json.dumps(result))
        self.assertEqual(next(r for r in result["records"] if r["record_uid"] == "record")["title"], "Service account")
        self.assertEqual(next(f for f in result["folders"] if f["folder_uid"] == "nested_folder")["folder_path"], "/Operations/Nested share")

    def test_empty_vault_is_explicit_success(self):
        params = types.SimpleNamespace(folder_cache={}, record_cache={}, subfolder_record_cache={})
        result = report.snapshot(params, [])
        self.assertEqual(len(result["folders"]), 1)
        self.assertEqual(result["records"], [])

    def test_invalid_snapshots_fail_instead_of_erasing_inventory(self):
        params = self.params()
        params.folder_cache["shared"].parent_uid = "child"
        with self.assertRaises(ValueError):
            report.snapshot(params, [])
        params = self.params()
        params.subfolder_record_cache["child"].add("missing")
        with self.assertRaises(ValueError):
            report.snapshot(params, [])
        params = self.params()
        params.record_cache["record"]["data_unencrypted"] = "SECRET-malformed"
        with self.assertRaises(ValueError):
            report.snapshot(params, [])

    def test_commander_output_and_errors_are_suppressed(self):
        def collect(config):
            print("SECRET-BANNER")
            print("SECRET-STDERR", file=sys.stderr)
            return report.snapshot(self.params(), [])
        with patch.object(report, "collect_report", collect), patch.object(sys, "argv", ["metadata", "--config", "unused"]):
            stdout, stderr = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                self.assertEqual(report.main(), 0)
            self.assertEqual(stderr.getvalue(), "")
            self.assertNotIn("SECRET", stdout.getvalue())
            self.assertEqual(json.loads(stdout.getvalue())["schema_version"], 1)
        for error, code in [(ValueError("SECRET-EXCEPTION"), 1), (PermissionError("SECRET-ACCOUNT"), 2)]:
            with patch.object(report, "collect_report", side_effect=error), patch.object(sys, "argv", ["metadata", "--config", "unused"]):
                stdout, stderr = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                    self.assertEqual(report.main(), code)
                self.assertEqual(stdout.getvalue(), "")
                self.assertNotIn("SECRET", stderr.getvalue())

    def test_requires_real_session_before_sync(self):
        params = self.params()
        params.session_token = None
        keeper = types.ModuleType("keepercommander")
        keeper.api = types.SimpleNamespace(sync_down=lambda p: self.fail("must not sync without session"))
        entry = types.ModuleType("keepercommander.__main__")
        entry.get_params_from_config = lambda config: params
        utils = types.ModuleType("keepercommander.commands.utils")
        utils.LoginStatusCommand = lambda: types.SimpleNamespace(execute=lambda p: None)
        register = types.ModuleType("keepercommander.commands.register")
        register.ShareReportCommand = types.SimpleNamespace(sf_report=lambda *a, **kw: "[]")
        with patch.dict(sys.modules, {"keepercommander": keeper, "keepercommander.__main__": entry,
                                     "keepercommander.commands.utils": utils, "keepercommander.commands.register": register}):
            with self.assertRaises(PermissionError):
                report.collect_report("unused")
            params.session_token = "synthetic-session"
            keeper.api.sync_down = lambda p: None
            self.assertEqual(report.collect_report("unused")["schema_version"], 1)


if __name__ == "__main__":
    unittest.main()
