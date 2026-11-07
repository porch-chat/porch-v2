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
    # Control-plane-only LiveKit stand-in: real tickets/leases/flags, no media.
    lk = free_port()
    processes.append(subprocess.Popen([sys.executable, str(ROOT / "scripts/livekit-stub.py"), str(lk)]))
    env.update(PORCH_VOICE_URL=f"ws://127.0.0.1:{lk}", PORCH_VOICE_INTERNAL_URL=f"http://127.0.0.1:{lk}",
               LIVEKIT_API_KEY="fixturekey", LIVEKIT_API_SECRET="fixturesecret-0123456789abcdef0123456789")
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
        if "--screenshots" in sys.argv:
            display = ":" + str(100 + free_port() % 1000)
            env.update(DISPLAY=display, XDG_DATA_HOME=temp + "/app-data")
            xvfb = subprocess.Popen(["Xvfb", display, "-screen", "0", "1920x1080x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(xvfb)
            time.sleep(1)
            wm = subprocess.Popen(["openbox"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(wm)
            env["PORCH_ORIGIN"] = env["PORCH_URL"]
            open("/tmp/porch-commet-fixture.json", "w").write(json.dumps({"url": env["PORCH_URL"], "password": password, "display": display}))
            app = subprocess.Popen([ROOT / "apps/porch_commet/commet/build/linux/x64/debug/bundle/commet"], env=env, stdout=open("/tmp/commet-app.log", "w"), stderr=subprocess.STDOUT)
            processes.append(app)
            while os.path.exists("/tmp/commet-hold"):
                time.sleep(1)
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
