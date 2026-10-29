import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("launch-clean-scan.py")
spec = importlib.util.spec_from_file_location("launch_clean_scan", SCRIPT)
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)


class FakeChild:
    pid = 12345

    def wait(self):
        return 7

    def poll(self):
        return 7


class LaunchTest(unittest.TestCase):
    def prepare(self, batch):
        batch.mkdir()
        files = scan.paths(batch)
        files["binary"].write_bytes(b"fixed bsync copy")
        files["binary"].chmod(0o700)
        files["input"].write_bytes(b"exported HTML copy")
        files["dns_policy"].write_text(scan.json.dumps({
            "default_resolvers": ["cloudflare"], "china_resolvers": ["alidns"],
            "confirmation_resolvers": ["cloudflare", "alidns"],
        }))
        return files

    def test_command_preserves_fixed_safety_settings_and_explicit_route(self):
        files = scan.paths(Path("/batch"))
        direct = scan.command(files, None, 30, files["dns_policy"])
        self.assertNotIn("--proxy", direct)
        self.assertEqual(direct[direct.index("--timeout") + 1], "30")
        self.assertEqual(direct[direct.index("--dns-policy") + 1], "/batch/dns-policy.json")
        self.assertIn("--slow", direct)
        for option, value in [("--concurrency", "2"), ("--scope", "content"),
                              ("--secondary", "curl"), ("--limit", "0")]:
            self.assertEqual(direct[direct.index(option) + 1], value)
        proxy = scan.command(files, "http://localhost:6152")
        self.assertEqual(proxy[proxy.index("--proxy") + 1], "http://localhost:6152")
        self.assertEqual(proxy[proxy.index("--timeout") + 1], "10")

    def test_direct_verify_is_read_only_and_does_not_test_a_proxy(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            files = self.prepare(batch)
            with patch.object(scan, "validate_proxy") as proxy, patch.object(scan, "competing_scans", return_value=[]):
                with contextlib.redirect_stdout(io.StringIO()):
                    scan.verify_only(batch, scan.digest(files["binary"]), scan.digest(files["input"]),
                                     None, 30, files["dns_policy"], scan.digest(files["dns_policy"]))
            proxy.assert_not_called()
            self.assertFalse(files["request"].exists())

    def test_dns_backup_toggle_is_snapshotted_and_invalid_type_is_rejected(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            files = self.prepare(batch)
            files["dns_policy"].write_text('{"dns_fallback_on_error":"false"}')
            with self.assertRaisesRegex(RuntimeError, "boolean"):
                scan.network_request(files, None, dns_policy=files["dns_policy"])
            for enabled in (True, False):
                files["dns_policy"].write_text(scan.json.dumps({"dns_fallback_on_error": enabled}))
                request = scan.network_request(files, None, dns_policy=files["dns_policy"])
                self.assertEqual(request["dns_policy_sha256"], scan.digest(files["dns_policy"]))
            self.assertFalse(files["manifest"].exists())

    def test_worker_uses_saved_timeout_and_dns_without_running_scan(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            files = self.prepare(batch)
            dns_hash = scan.digest(files["dns_policy"])
            with patch.object(scan.subprocess, "run"):
                with contextlib.redirect_stdout(io.StringIO()):
                    scan.launch(batch, scan.digest(files["binary"]), scan.digest(files["input"]),
                                None, 30, files["dns_policy"], dns_hash)
            request = scan.json.loads(files["request"].read_text())
            self.assertEqual(request["dns_policy_sha256"], dns_hash)
            with patch.object(scan, "validate_proxy") as proxy, patch.object(scan, "competing_scans", return_value=[]), \
                    patch.object(scan.subprocess, "Popen", return_value=FakeChild()) as popen, \
                    patch.object(scan, "remove_service"):
                self.assertEqual(scan.worker(batch), 7)
            proxy.assert_not_called()
            executed = popen.call_args.args[0]
            self.assertEqual(executed[executed.index("--timeout") + 1], "30")
            self.assertNotIn("--proxy", executed)
            manifest = scan.json.loads(files["manifest"].read_text())
            self.assertEqual(manifest["network"]["dns_policy_sha256"], dns_hash)
            self.assertEqual(manifest["network"]["route"], "direct")
            self.assertEqual(manifest["network"]["timeout_secs"], 30)
            self.assertIsNone(manifest["deadline_at"])

    def test_changed_dns_policy_never_starts_child(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            files = self.prepare(batch)
            with patch.object(scan.subprocess, "run"):
                with contextlib.redirect_stdout(io.StringIO()):
                    scan.launch(batch, scan.digest(files["binary"]), scan.digest(files["input"]),
                                None, dns_policy=files["dns_policy"])
            files["dns_policy"].write_text("{}")
            with patch.object(scan.subprocess, "Popen") as popen, patch.object(scan, "remove_service"):
                self.assertEqual(scan.worker(batch), 1)
            popen.assert_not_called()
            self.assertIn("DNS policy SHA-256 mismatch", scan.json.loads(files["manifest"].read_text())["error"])
            self.assertFalse(files["db"].exists())

    def test_invalid_timeout_hash_or_confirmation_refuses_before_request(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            files = self.prepare(batch)
            for timeout in (4, 61, True):
                with self.assertRaisesRegex(RuntimeError, "timeout"):
                    scan.launch(batch, scan.digest(files["binary"]), scan.digest(files["input"]), None, timeout)
            with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                scan.launch(batch, scan.digest(files["binary"]), scan.digest(files["input"]), None,
                            dns_policy=files["dns_policy"], dns_policy_hash="0" * 64)
            files["dns_policy"].write_text('{"confirmation_resolvers":["cloudflare","cloudflare"]}')
            with self.assertRaisesRegex(RuntimeError, "distinct"):
                scan.launch(batch, scan.digest(files["binary"]), scan.digest(files["input"]), None,
                            dns_policy=files["dns_policy"])
            self.assertFalse(files["request"].exists())

    def test_verify_only_is_read_only(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            batch.mkdir()
            files = scan.paths(batch)
            files["binary"].write_bytes(b"fixed bsync copy")
            files["binary"].chmod(0o700)
            files["input"].write_bytes(b"exported HTML copy")
            with patch.object(scan, "validate_proxy"), patch.object(scan, "competing_scans", return_value=[]):
                with contextlib.redirect_stdout(io.StringIO()):
                    scan.verify_only(batch, scan.digest(files["binary"]),
                                     scan.digest(files["input"]), "http://127.0.0.1:6152")
            self.assertFalse(files["request"].exists())
            self.assertFalse(files["manifest"].exists())
            self.assertFalse((batch.parent / ".bookmark-health-scan.lock").exists())

    def test_one_shot_and_hash_guard_without_running_scan(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            batch.mkdir()
            files = scan.paths(batch)
            files["binary"].write_bytes(b"fixed bsync copy")
            files["binary"].chmod(0o700)
            files["input"].write_bytes(b"exported HTML copy")
            binary_hash = scan.digest(files["binary"])
            input_hash = scan.digest(files["input"])
            with patch.object(scan, "validate_proxy"), patch.object(scan.subprocess, "run") as submit:
                with contextlib.redirect_stdout(io.StringIO()):
                    scan.launch(batch, binary_hash, input_hash, "http://127.0.0.1:6152")
            self.assertIn("submit", submit.call_args.args[0])
            with self.assertRaises(FileExistsError):
                with patch.object(scan, "validate_proxy"), patch.object(scan.subprocess, "run"):
                    scan.launch(batch, binary_hash, input_hash, "http://127.0.0.1:6152")

            with patch.object(scan, "validate_proxy"), patch.object(scan, "competing_scans", return_value=[]), \
                    patch.object(scan.subprocess, "Popen", return_value=FakeChild()) as popen, \
                    patch.object(scan, "remove_service"):
                self.assertEqual(scan.worker(batch), 7)
                self.assertEqual(scan.worker(batch), 1)
            popen.assert_called_once()
            self.assertEqual(scan.json.loads(files["receipt"].read_text())["process_exit_code"], 7)
            self.assertEqual(scan.json.loads(files["manifest"].read_text())["status"], "failed")

    def test_changed_input_never_starts_child(self):
        with tempfile.TemporaryDirectory(dir=SCRIPT.parents[1]) as temporary:
            batch = Path(temporary) / "fresh-batch"
            batch.mkdir()
            files = scan.paths(batch)
            files["binary"].write_bytes(b"fixed bsync copy")
            files["binary"].chmod(0o700)
            files["input"].write_bytes(b"first copy")
            with patch.object(scan, "validate_proxy"), patch.object(scan.subprocess, "run"):
                with contextlib.redirect_stdout(io.StringIO()):
                    scan.launch(batch, scan.digest(files["binary"]), scan.digest(files["input"]),
                                "http://127.0.0.1:6152")
            files["input"].write_bytes(b"changed copy")
            with patch.object(scan.subprocess, "Popen") as popen, patch.object(scan, "remove_service"):
                self.assertEqual(scan.worker(batch), 1)
            popen.assert_not_called()
            self.assertFalse(files["db"].exists())


if __name__ == "__main__":
    unittest.main()
