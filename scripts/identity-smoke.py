#!/usr/bin/env python3
"""Disposable Postgres only. Prove actual porch-cli against a local HTTP server."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import time
import urllib.request

root = Path(__file__).resolve().parent.parent
env = os.environ.copy()
assert env.get("DATABASE_URL"), "DATABASE_URL must point at disposable Postgres"
password = secrets.token_hex(24)
env.update(PORCH_BOOTSTRAP_USERNAME="smoke_admin", PORCH_BOOTSTRAP_PASSWORD=password)
server_bin = str(root / "target/debug/porch-server")
cli_bin = str(root / "target/debug/porch-cli")
# One-shot bootstrap must be refused if the DB already contains a user.
subprocess.run([server_bin, "bootstrap-admin"], env=env, check=True, capture_output=True)
assert subprocess.run([server_bin, "bootstrap-admin"], env=env, capture_output=True).returncode != 0
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
env.update(PORCH_BIND=f"127.0.0.1:{port}", PORCH_URL=f"http://127.0.0.1:{port}")
# Bootstrap variables are not passed to the long-running server.
env.pop("PORCH_BOOTSTRAP_USERNAME")
env.pop("PORCH_BOOTSTRAP_PASSWORD")
server = subprocess.Popen([server_bin], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

def cli(*args, fail=False, **extra):
    result = subprocess.run([cli_bin, *map(str, args)], env={**env, **extra}, capture_output=True, text=True, timeout=20)
    if fail:
        assert result.returncode != 0, args
        return None
    assert result.returncode == 0, (args, result.stderr)
    return json.loads(result.stdout) if result.stdout.startswith(("{", "[")) else result.stdout.strip()

try:
    for _ in range(100):
        try:
            with urllib.request.urlopen(env["PORCH_URL"] + "/health", timeout=1) as response:
                assert response.status == 200
            break
        except OSError:
            if server.poll() is not None:
                raise RuntimeError("server stopped")
            time.sleep(0.05)
    else:
        raise RuntimeError("server did not become ready")
    admin = cli("login", PORCH_USERNAME="smoke_admin", PORCH_PASSWORD=password)
    admin_token = admin["token"]
    # Exercise the actual CLI gateway consumer, not just a test-only WS client.
    listener = subprocess.Popen([cli_bin, "gateway"], env={**env, "PORCH_TOKEN": admin_token},
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        listener.communicate(timeout=1)
        raise AssertionError("gateway listener exited unexpectedly")
    except subprocess.TimeoutExpired:
        listener.kill()
        output, errors = listener.communicate(timeout=5)
    events = [json.loads(line) for line in output.splitlines()]
    assert events[0]["op"] == "hello", errors
    assert events[1]["op"] == "ready" and not events[1]["resumed"], errors
    assert any(event["op"] == "heartbeat_ack" for event in events), errors
    assert cli("me", PORCH_TOKEN=admin_token)["admin"]
    invite = cli("invite-create", 2, 3600, PORCH_TOKEN=admin_token)
    user = cli("register", PORCH_USERNAME="smoke_user", PORCH_PASSWORD=password, PORCH_INVITE=invite["code"])
    user_token = user["token"]
    assert not user["user"]["admin"]
    cli("invite-create", 1, 3600, PORCH_TOKEN=user_token, fail=True)
    other = cli("login", PORCH_USERNAME="smoke_user", PORCH_PASSWORD=password)
    sessions = cli("sessions", PORCH_TOKEN=user_token)
    assert len(sessions) == 2
    cli("revoke-session", next(s["id"] for s in sessions if s["current"]), PORCH_TOKEN=other["token"])
    cli("me", PORCH_TOKEN=user_token, fail=True)
    cli("logout-all", PORCH_TOKEN=other["token"])
    cli("me", PORCH_TOKEN=other["token"], fail=True)
    assert cli("invites", PORCH_TOKEN=admin_token)[0]["uses"] == 1
    cli("invite-revoke", invite["invite"]["id"], PORCH_TOKEN=admin_token)
    cli("register", PORCH_USERNAME="smoke_denied", PORCH_PASSWORD=password, PORCH_INVITE=invite["code"], fail=True)
    cli("logout", PORCH_TOKEN=admin_token)
    cli("me", PORCH_TOKEN=admin_token, fail=True)
    print("CLI identity smoke passed, bootstrap/register/login/invites/session revocation/logout proven over HTTP")
finally:
    server.terminate()
    server.communicate(timeout=10)
