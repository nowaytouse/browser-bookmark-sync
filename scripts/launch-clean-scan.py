#!/usr/bin/env python3
"""Submit one independent, copy-only bookmark check on macOS.

Prepare a fresh batch first: copy a fixed bsync binary into it, export
``input.html`` there, then run this script with both files' SHA-256 hashes.
Never retry a batch; make a new directory for a new scan.
"""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
from datetime import datetime, timezone
from urllib.parse import urlsplit


def stamp():
    return datetime.now(timezone.utc).isoformat()


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def write_new(path, value):
    with path.open("x", encoding="utf-8") as output:
        json.dump(value, output, indent=2)
        output.write("\n")


def write_atomic(path, value):
    temporary = path.with_name(path.name + f".{os.getpid()}.tmp")
    with temporary.open("x", encoding="utf-8") as output:
        json.dump(value, output, indent=2)
        output.write("\n")
    os.replace(temporary, path)


def paths(batch):
    return {
        "binary": batch / "bsync",
        "input": batch / "input.html",
        "db": batch / "bookmark-health.sqlite",
        "health": batch / "health",
        "log": batch / "health" / "browser-sync.log",
        "request": batch / "launch-request.json",
        "manifest": batch / "run-manifest.json",
        "receipt": batch / "process-exit.json",
    }


def command(files, proxy):
    return [
        str(files["binary"]), "--log-file", str(files["log"]), "check",
        "--file", str(files["input"]), "--slow", "--scope", "content",
        "--concurrency", "2", "--timeout", "10", "--secondary", "curl",
        "--limit", "0", "--proxy", proxy, "--history-db", str(files["db"]),
        "--export-dir", str(files["health"]),
    ]


def validate_files(files, expected):
    for name in ("binary", "input"):
        path = files[name]
        if not re.fullmatch(r"[0-9a-f]{64}", expected[name]):
            raise RuntimeError(f"invalid {name} SHA-256 hash")
        if not path.is_file() or path.is_symlink() or digest(path) != expected[name]:
            raise RuntimeError(f"{name} missing, linked, or SHA-256 mismatch: {path}")
    if not os.access(files["binary"], os.X_OK):
        raise RuntimeError("bsync copy is not executable")
    if any(path.exists() for path in (files["db"], files["receipt"],
                                      Path(str(files["db"]) + "-wal"),
                                      Path(str(files["db"]) + "-shm"))):
        raise RuntimeError("batch has existing database state or exit receipt")


def validate_proxy(proxy):
    parsed = urlsplit(proxy)
    if (parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "localhost")
            or not parsed.port or parsed.username or parsed.password or parsed.path not in ("", "/")
            or parsed.query or parsed.fragment):
        raise RuntimeError("proxy must be a local HTTP proxy URL")
    with socket.create_connection((parsed.hostname, parsed.port), timeout=2):
        pass


def competing_scans():
    result = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True,
                            text=True, check=True)
    found = []
    for line in result.stdout.splitlines():
        match = re.match(r"\s*(\d+)\s+(.+)", line)
        if not match or int(match[1]) == os.getpid():
            continue
        if (re.search(r"(?:^|\s)(?:\S*/)?(?:bsync|browser-bookmark-sync)(?=\s)", match[2])
                and re.search(r"(?:^|\s)check(?:\s|$)", match[2])):
            found.append(int(match[1]))
    return found


def remove_service(label):
    subprocess.run(["launchctl", "remove", label], capture_output=True, check=False)


def label_for(batch):
    return "local.browser-sync.check." + hashlib.sha256(str(batch).encode()).hexdigest()[:16]


def worker(batch):
    if not batch.is_dir() or batch.is_symlink():
        raise RuntimeError("batch must be an existing ordinary directory")
    files = paths(batch)
    request = json.loads(files["request"].read_text(encoding="utf-8"))
    label = request["label"]
    if label != label_for(batch):
        raise RuntimeError("launch label does not match batch")
    manifest = {"status": "starting", "supervisor_pid": os.getpid(),
                "supervisor_ppid": os.getppid(), "run_until_complete": True,
                "deadline_at": None, "baseline_actual_requests": 0,
                "baseline_saved_targets": 0,
                "submitted_at": request["submitted_at"], "started_at": stamp(),
                "command": command(files, request["proxy"]),
                "binary_sha256": request["binary"], "input_sha256": request["input"],
                "launch_label": label}
    try:
        write_new(files["manifest"], manifest)
    except FileExistsError:
        # launchd may re-enter the supervisor; a used batch must never start again.
        remove_service(label)
        return 1
    exit_code = 1
    child = None
    termination_signal = None

    def forward_signal(number, _frame):
        nonlocal termination_signal
        termination_signal = signal.Signals(number).name
        if child is None:
            raise RuntimeError(f"supervisor received {termination_signal} before child start")
        if child.poll() is None:
            child.send_signal(number)

    old_sigterm = signal.signal(signal.SIGTERM, forward_signal)
    old_sigint = signal.signal(signal.SIGINT, forward_signal)
    try:
        lock_path = batch.parent / ".bookmark-health-scan.lock"
        # ponytail: one global scan lock; use per-host locks only if parallel scans become necessary.
        with lock_path.open("a+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            validate_files(files, request)
            validate_proxy(request["proxy"])
            others = competing_scans()
            if others:
                raise RuntimeError(f"existing bsync check process(es): {others}")
            files["health"].mkdir(mode=0o700, exist_ok=False)
            child = subprocess.Popen(manifest["command"], cwd=batch,
                                     stdin=subprocess.DEVNULL)
            manifest.update(status="running", child_pid=child.pid,
                            child_ppid=os.getpid(), scan_started_at=stamp())
            write_atomic(files["manifest"], manifest)
            exit_code = child.wait()
            manifest.update(status="exited_pending_coverage_verification" if exit_code == 0 else "failed",
                            finished_at=stamp(), process_exit_code=exit_code,
                            termination_signal=termination_signal)
            write_atomic(files["manifest"], manifest)
    except Exception as error:
        manifest.update(status="failed", finished_at=stamp(), error=str(error),
                        process_exit_code=exit_code, termination_signal=termination_signal)
        write_atomic(files["manifest"], manifest)
    finally:
        if child and child.poll() is None:
            exit_code = child.wait()
        write_new(files["receipt"], {"process_exit_code": exit_code,
                                     "recorded_at": stamp(), "child_pid": child.pid if child else None,
                                     "termination_signal": termination_signal})
        signal.signal(signal.SIGTERM, old_sigterm)
        signal.signal(signal.SIGINT, old_sigint)
        remove_service(label)
    return exit_code


def launch(batch, binary_hash, input_hash, proxy):
    if not batch.is_dir() or batch.is_symlink():
        raise RuntimeError("batch must be an existing ordinary directory")
    files = paths(batch)
    expected = {"binary": binary_hash, "input": input_hash}
    validate_files(files, expected)
    if files["health"].exists() or files["manifest"].exists():
        raise RuntimeError("batch already used")
    validate_proxy(proxy)
    label = label_for(batch)
    request = {**expected, "proxy": proxy, "label": label, "submitted_at": stamp()}
    write_new(files["request"], request)
    subprocess.run(["launchctl", "submit", "-l", label,
                    "-o", str(batch / "supervisor.stdout.log"),
                    "-e", str(batch / "supervisor.stderr.log"), "--",
                    sys.executable, str(Path(__file__).resolve()), "--worker", str(batch)],
                   check=True)
    print(json.dumps({"batch": str(batch), "label": label, "manifest": str(files["manifest"])}))


def verify_only(batch, binary_hash, input_hash, proxy):
    if not batch.is_dir() or batch.is_symlink():
        raise RuntimeError("batch must be an existing ordinary directory")
    files = paths(batch)
    validate_files(files, {"binary": binary_hash, "input": input_hash})
    if files["health"].exists() or files["request"].exists() or files["manifest"].exists():
        raise RuntimeError("batch already used")
    validate_proxy(proxy)
    others = competing_scans()
    if others:
        raise RuntimeError(f"existing bsync check process(es): {others}")
    print(json.dumps({"batch": str(batch), "verified": True}))


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--verify-only", action="store_true")
    parser.add_argument("--batch", type=Path)
    parser.add_argument("--binary-sha256")
    parser.add_argument("--input-sha256")
    parser.add_argument("--proxy", default="http://127.0.0.1:6152")
    args = parser.parse_args()
    try:
        if args.worker:
            return worker(args.worker.absolute())
        if not args.batch or not args.binary_sha256 or not args.input_sha256:
            parser.error("--batch, --binary-sha256, and --input-sha256 are required")
        batch = args.batch.absolute()
        if args.verify_only:
            verify_only(batch, args.binary_sha256, args.input_sha256, args.proxy)
        else:
            launch(batch, args.binary_sha256, args.input_sha256, args.proxy)
        return 0
    except Exception as error:
        print(f"scan launch refused: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
