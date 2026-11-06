#!/usr/bin/env python3
"""Disposable local server fixture. No production access or committed account credentials.
DATABASE_URL must be a local Postgres admin connection; creates/drops only its random database.
Runs actual generated Dart/native FFI integration. --screenshots also drives the real Linux app.
"""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
env = os.environ.copy()
base = urllib.parse.urlparse(env["DATABASE_URL"])
if base.hostname not in ("127.0.0.1", "localhost"):
    raise SystemExit("Fixture requires loopback Postgres")
dbname = "porch_flutter_" + secrets.token_hex(6)
admin = base._replace(path="/postgres").geturl()
env["DATABASE_URL"] = base._replace(path="/" + dbname).geturl()
password = secrets.token_hex(24)
BIN = ROOT / "target/debug"
created = False
processes = []


def command(args, **extra):
    result = subprocess.run(args, env={**env, **extra}, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError("Fixture command failed: " + str(args[0]))
    return result.stdout


def cli(*args, **extra):
    return json.loads(command([BIN / "porch-cli", *map(str, args)], **extra))


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


try:
    command(["psql", admin, "-qc", f"CREATE DATABASE {dbname}"])
    created = True
    command([BIN / "porch-server", "bootstrap-admin"],
            PORCH_BOOTSTRAP_USERNAME="sam", PORCH_BOOTSTRAP_PASSWORD=password)
    env.update(PORCH_BIND=f"127.0.0.1:{free_port()}")
    env["PORCH_URL"] = "http://" + env["PORCH_BIND"]
    with tempfile.TemporaryDirectory(prefix="porch-flutter-fixture-") as temp:
        env.update(PORCH_STORAGE_ROOT=temp + "/files")
        server = subprocess.Popen([BIN / "porch-server"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        processes.append(server)
        for _ in range(100):
            try:
                urllib.request.urlopen(env["PORCH_URL"] + "/health", timeout=1)
                break
            except OSError:
                time.sleep(.1)
        auth = cli("login", PORCH_USERNAME="sam", PORCH_PASSWORD=password)
        env["PORCH_TOKEN"] = auth["token"]
        community = cli("community-create", "Friends")["id"]

        def mutate(**body):
            return cli("community-mutate", community, PORCH_COMMAND=json.dumps(body))["id"]

        category = mutate(op="create_category", name="Rooms", position=0)
        for index, name in enumerate(["general", "plans", "random"]):
            mutate(op="create_channel", name=name, kind="text", category_id=category, position=index, synced=True)
        mutate(op="create_channel", name="hangout", kind="voice", category_id=category, position=3, synced=True)
        invite = cli("invite-create", 2, 3600)["code"]
        join = cli("community-invite-create", community, 2, 3600)["code"]
        maya = cli("register", PORCH_USERNAME="maya", PORCH_PASSWORD=password, PORCH_INVITE=invite)
        cli("community-join", PORCH_TOKEN=maya["token"], PORCH_COMMUNITY_INVITE=join)
        heads = cli("conversations")
        head = next(h for h in heads if h.get("community_id") == community and h["name"] == "general")
        for n, (token, text) in enumerate([
            (maya["token"], "Here's the photo from the hike!"),
            (auth["token"], "Looks amazing. I can do the second weekend."),
            (maya["token"], "Perfect. Let us plan a weekend trip?"),
        ]):
            cli("message-send", head["id"], PORCH_TOKEN=token,
                PORCH_MESSAGE=json.dumps({"nonce": f"flutter_fixture_{n:020}", "content": {"format": "markdown", "text": text}, "reply_to": None}))
        dm = cli("dm-create", PORCH_COMMAND=json.dumps({"participants": [maya["user"]["id"]]}))
        cli("message-send", dm["id"], PORCH_TOKEN=maya["token"],
            PORCH_MESSAGE=json.dumps({"nonce": "flutter_fixture_dm_00000001", "content": {"format": "markdown", "text": "Hey! Want to grab dinner this week?"}, "reply_to": None}))
        native_env = {**env,
            "PORCH_FIXTURE_ORIGIN": env["PORCH_URL"], "PORCH_FIXTURE_USERNAME": "sam",
            "PORCH_FIXTURE_PASSWORD": password, "PORCH_FIXTURE_COMMUNITY": str(community),
            "PORCH_BRIDGE_LIBRARY": str(BIN / "libporch_flutter.so")}
        result = subprocess.run(["flutter", "test", "test/native_bridge_test.dart"], cwd=ROOT / "apps/porch_flutter", env=native_env)
        if result.returncode:
            raise RuntimeError("Generated Dart/native bridge integration failed")
        print("Generated Dart/native bridge integration passed")
        if "--screenshots" in sys.argv:
            display = ":" + str(100 + free_port() % 1000)
            env.update(DISPLAY=display, XDG_DATA_HOME=temp + "/app-data")
            xvfb = subprocess.Popen(["Xvfb", display, "-screen", "0", "1924x1200x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(xvfb)
            time.sleep(1)
            wm = subprocess.Popen(["openbox"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(wm)
            app = subprocess.Popen([ROOT / "apps/porch_flutter/build/linux/x64/release/bundle/porch_flutter"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(app)
            wid = command(["xdotool", "search", "--sync", "--name", "^Porch$"]).split()[0]
            command(["xdotool", "windowmove", wid, "0", "0", "windowsize", wid, "1920", "1080"])
            time.sleep(2)
            evidence = ROOT / "docs/evidence"
            evidence.mkdir(exist_ok=True)

            def shot(name):
                command(["import", "-window", wid, str(evidence / f"flutter-{name}.png")])

            def click(x, y):
                # Window-relative coordinates, translated by actual X geometry.
                geometry = command(["xdotool", "getwindowgeometry", "--shell", wid])
                xy = dict(line.split("=", 1) for line in geometry.splitlines() if "=" in line)
                command(["xdotool", "mousemove", str(int(xy["X"]) + x), str(int(xy["Y"]) + y), "click", "1"])

            def type_text(value):
                subprocess.run(["xdotool", "type", "--delay", "25", "--clearmodifiers", "--file", "-"], input=value, text=True, env=env, check=True)

            shot("signin")
            # Coordinates verified from the actual rendered login capture at 1920x1080.
            for y, value in [(493, env["PORCH_URL"]), (556, "sam"), (620, password)]:
                click(960, y)
                command(["xdotool", "key", "ctrl+a"])
                time.sleep(.2)
                type_text(value)
                time.sleep(.2)
            click(960, 684)
            time.sleep(5)
            shot("home")
            click(510, 27)
            time.sleep(2)
            click(110, 144)
            time.sleep(2)
            shot("group")
            click(45, 27)
            time.sleep(1)
            click(110, 142)
            time.sleep(2)
            shot("dm")
            print("Real local fixture screenshots captured; inspect before claiming")
finally:
    for process in reversed(processes):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
    if created:
        command(["psql", admin, "-qc", f"DROP DATABASE {dbname} WITH (FORCE)"])
