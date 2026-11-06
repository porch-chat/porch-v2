#!/usr/bin/env python3
"""Run under xvfb-run with Openbox installed; DATABASE_URL must be empty/disposable.

Real server, real native GPUI rendering, real keyboard send. Never use a user DB.
Only generated fixture content appears in the optional screenshot.
"""
import json
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request

root = Path(__file__).resolve().parent.parent
env = os.environ.copy()
assert env.get("DATABASE_URL"), "DATABASE_URL must point at empty disposable Postgres"
assert env.get("DISPLAY"), "Run under xvfb-run -s '-screen 0 1440x1000x24'"
password = secrets.token_hex(24)
server_bin = str(root / "target/debug/porch-server")
cli_bin = str(root / "target/debug/porch-cli")
desktop_bin = str(root / "target/debug/porch-desktop")
# Refuses nonempty user databases before starting anything graphical.
subprocess.run(
    [server_bin, "bootstrap-admin"],
    env={**env, "PORCH_BOOTSTRAP_USERNAME": "desktop_smoke", "PORCH_BOOTSTRAP_PASSWORD": password},
    capture_output=True, check=True, timeout=20,
)
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
env.update(PORCH_BIND=f"127.0.0.1:{port}", PORCH_URL=f"http://127.0.0.1:{port}")
server = subprocess.Popen([server_bin], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
wm = None
desktop = None


def cli(*args, **extra):
    result = subprocess.run([cli_bin, *map(str, args)], env={**env, **extra},
                            capture_output=True, text=True, check=True, timeout=20)
    return json.loads(result.stdout)


def ready():
    try:
        with urllib.request.urlopen(env["PORCH_URL"] + "/health", timeout=1) as r:
            return r.status == 200
    except OSError:
        return False


def until(test, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if test():
            return
        if server.poll() is not None or (desktop is not None and desktop.poll() is not None):
            raise AssertionError("smoke process stopped before acceptance")
        time.sleep(0.05)
    raise AssertionError("smoke readiness/acceptance deadline exceeded")


try:
    until(ready)
    admin = cli("login", PORCH_USERNAME="desktop_smoke", PORCH_PASSWORD=password)
    token = admin["token"]
    heads = cli("conversations", PORCH_TOKEN=token)
    assert heads
    for i, text in enumerate([
        "Pull up. Talk awhile.",
        "A real local server, native client, cached history and durable sends.",
        "No remote images or embedded browser.",
    ]):
        cli("message-send", heads[0]["id"], PORCH_TOKEN=token,
            PORCH_MESSAGE=json.dumps({"nonce": f"native_smoke_nonce_{i:08}",
                                     "content": {"format": "markdown", "text": text}, "reply_to": None}))
    with tempfile.TemporaryDirectory(prefix="porch-native-smoke-") as data:
        ui = {**env, "PORCH_DATA_DIR": data, "PORCH_SMOKE_SECONDS": "35"}
        ui.pop("PORCH_TOKEN", None)
        wm = subprocess.Popen(["openbox"], env=ui, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with tempfile.TemporaryFile() as log:
            desktop = subprocess.Popen([desktop_bin], env=ui, stdout=log, stderr=log)
            subprocess.run(["xdotool", "search", "--sync", "--name", "^Porch$"],
                           env=ui, check=True, timeout=20, stdout=subprocess.DEVNULL)
            time.sleep(2)
            for y, value in [(509, env["PORCH_URL"]), (562, "desktop_smoke"), (614, password)]:
                subprocess.run(["xdotool", "mousemove", "720", str(y), "click", "1", "key", "ctrl+a"],
                               env=ui, check=True, timeout=10)
                # Fixture credentials travel through stdin, not process argv.
                subprocess.run(["xdotool", "type", "--clearmodifiers", "--file", "-"],
                               input=value, text=True, env=ui, check=True, timeout=10)
            subprocess.run(["xdotool", "mousemove", "720", "665", "click", "1"],
                           env=ui, check=True, timeout=10)

            def count():
                paths = list(Path(data).glob("*.sqlite3"))
                if not paths:
                    return 0
                try:
                    with sqlite3.connect(paths[0]) as db:
                        return db.execute("SELECT count(*) FROM messages").fetchone()[0]
                except sqlite3.OperationalError:
                    return 0

            until(lambda: count() >= 3)
            # Initial frame must be painted before interacting. Fixed 1440x1000
            # Xvfb resolution, centered 1360x860 window, default preferences.
            time.sleep(2)
            subprocess.run(["xdotool", "mousemove", "780", "843", "click", "1",
                            "type", "--clearmodifiers", "Sent through the native keyboard composer."],
                           env=ui, check=True, timeout=10)
            subprocess.run(["xdotool", "key", "Return"], env=ui, check=True, timeout=10)
            until(lambda: count() >= 4)
            # Verify durable remote delivery as well, not just an optimistic row.
            def delivered():
                messages = cli("messages", heads[0]["id"], PORCH_TOKEN=token)
                return any(m.get("content", {}).get("text", "").strip() == "Sent through the native keyboard composer."
                           for m in messages["messages"])
            until(delivered)
            screenshot = os.environ.get("PORCH_SMOKE_SCREENSHOT")
            if screenshot:
                time.sleep(0.5)
                subprocess.run(["import", "-window", "root", screenshot], env=ui, check=True, timeout=10)
            desktop.wait(timeout=40)
            assert desktop.returncode == 0, "desktop did not exit cleanly"
    print("Native GPUI startup, interactive login, keyboard composer and remote delivery passed.")
finally:
    for process in (desktop, wm, server):
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
