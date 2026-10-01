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
