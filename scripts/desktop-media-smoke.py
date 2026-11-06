#!/usr/bin/env python3
"""Headless native attachment cards + explicit authenticated poster rendering.
Requires DISPLAY and a fresh disposable DATABASE_URL, never user data.
Does not claim platform picker/save-dialog or video playback acceptance.
"""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import urllib.request

root = Path(__file__).resolve().parent.parent
env = os.environ.copy()
assert env.get("DATABASE_URL") and env.get("DISPLAY")
password = secrets.token_hex(24)
binary = root / "target/debug"
subprocess.run([binary / "porch-server", "bootstrap-admin"],
               env={**env, "PORCH_BOOTSTRAP_USERNAME": "media_smoke", "PORCH_BOOTSTRAP_PASSWORD": password},
               capture_output=True, check=True, timeout=20)
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
env.update(PORCH_BIND=f"127.0.0.1:{port}", PORCH_URL=f"http://127.0.0.1:{port}",
           PORCH_FFMPEG_BIN="/usr/bin/ffmpeg", PORCH_PRLIMIT_BIN="/usr/bin/prlimit")
server = desktop = wm = None


def cli(*args, **extra):
    result = subprocess.run([binary / "porch-cli", *map(str, args)],
                            env={**env, **extra}, capture_output=True, text=True, check=True, timeout=20)
    return json.loads(result.stdout)


def until(test, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if test():
            return
        if server.poll() is not None or (desktop and desktop.poll() is not None):
            raise AssertionError("process exited")
        time.sleep(0.05)
    raise AssertionError("native media deadline")


def ready():
    try:
        with urllib.request.urlopen(env["PORCH_URL"] + "/health", timeout=1) as r:
            return r.status == 200
    except OSError:
        return False


with tempfile.TemporaryDirectory(prefix="porch-media-smoke-") as directory:
    env.update(PORCH_STORAGE_ROOT=directory + "/storage")
    try:
        server = subprocess.Popen([binary / "porch-server"], env=env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        until(ready)
        token = cli("login", PORCH_USERNAME="media_smoke", PORCH_PASSWORD=password)["token"]
        env.update(PORCH_TOKEN=token)
        conversation = cli("conversations")[0]["id"]
        poster = root / "crates/porch-server/tests/fixtures/poster.mp4"
        upload = cli("file-upload", conversation, poster)
        message = cli("message-send", conversation,
                      PORCH_MESSAGE=json.dumps({"nonce": "native_media_fixture_00001",
                          "content": {"format": "markdown", "text": "A little porch light, delivered privately."},
                          "attachments": [upload["id"]], "reply_to": None}))
        assert message["attachments"][0]["mime"] == "video/mp4"
        env.update(PORCH_DATA_DIR=directory + "/cache", PORCH_SMOKE_SECONDS="25")
        wm = subprocess.Popen(["openbox"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with tempfile.TemporaryFile() as log:
            desktop = subprocess.Popen([binary / "porch-desktop"], env=env, stdout=log, stderr=log)
            subprocess.run(["xdotool", "search", "--sync", "--name", "^Porch$"],
                           env=env, check=True, timeout=20, stdout=subprocess.DEVNULL)
            time.sleep(3)
            # The acceptance harness uses inspected fixed-size Xvfb coordinates.
            # Caller supplies the inspected Load preview point, not a blind guess.
            point = os.environ.get("PORCH_PREVIEW_CLICK")
            if point:
                x, y = point.split(",")
                subprocess.run(["xdotool", "mousemove", x, y, "click", "1"],
                               env=env, check=True, timeout=10)
                time.sleep(2)
            screenshot = os.environ.get("PORCH_MEDIA_SCREENSHOT", directory + "/media.png")
            subprocess.run(["import", "-window", "root", screenshot], env=env, check=True, timeout=10)
            if point:
                # Fixture is solid amber. These points are inside the inspected
                # poster area, not button/text chrome, so a missing image fails.
                import re
                for pixel in ("600,500", "750,550"):
                    color = subprocess.check_output(
                        ["convert", screenshot, "-format", "%[pixel:p{" + pixel + "}]", "info:"],
                        text=True, timeout=10)
                    rgb = tuple(map(int, re.findall(r"\d+", color)))
                    assert len(rgb) == 3 and all(abs(a-b) <= 4 for a,b in zip(rgb,(242,182,89))), color
            desktop.wait(timeout=35)
            assert desktop.returncode == 0
        print("Native attachment card rendered" + ("; clicked authenticated video poster verified." if point else ".")
              + " No picker/hardware acceptance implied.")
    finally:
        for process in (desktop, wm, server):
            if process and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
