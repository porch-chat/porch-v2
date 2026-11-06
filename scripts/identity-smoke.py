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
    # Phase 3 end-to-end commands, including permission-filtered visibility.
    official = cli("communities", PORCH_TOKEN=user_token)
    assert len(official) == 1 and official[0]["official"]
    assert cli("community", official[0]["id"], PORCH_TOKEN=user_token)["permissions"] & 2 == 0
    community = cli("community-create", "Smoke Club", PORCH_TOKEN=user_token)
    cid = community["id"]
    def mutate(token, **command):
        return cli("community-mutate", cid, PORCH_TOKEN=token, PORCH_COMMAND=json.dumps(command))
    category = mutate(user_token, op="create_category", name="Inside", position=0)["id"]
    channel = mutate(user_token, op="create_channel", name="clubhouse", kind="text", category_id=category, position=0, synced=True)["id"]
    voice = mutate(user_token, op="create_channel", name="porch swing", kind="voice", category_id=None, position=0, synced=False)["id"]
    ci = cli("community-invite-create", cid, 2, 3600, PORCH_TOKEN=user_token)
    cli("community-join", PORCH_TOKEN=admin_token, PORCH_COMMUNITY_INVITE=ci["code"])
    assert len(cli("community", cid, PORCH_TOKEN=admin_token)["members"]) == 2
    conversations = cli("conversations", PORCH_TOKEN=user_token)
    conversation = next(c["id"] for c in conversations if c["community_id"] == cid)
    body = {"nonce":"smoke_message_nonce_0001", "content":{"format":"markdown","text":"**hi** searchable"}, "reply_to":None, "mentions":[admin["user"]["id"]]}
    message = cli("message-send", conversation, PORCH_TOKEN=user_token, PORCH_MESSAGE=json.dumps(body))
    assert cli("message-send", conversation, PORCH_TOKEN=user_token, PORCH_MESSAGE=json.dumps(body))["id"] == message["id"]
    assert cli("messages", conversation, PORCH_TOKEN=admin_token)["messages"][0]["id"] == message["id"]
    assert cli("message-search", conversation, "searchable", PORCH_TOKEN=admin_token)["messages"][0]["id"] == message["id"]
    def message_mutate(token, **command):
        return cli("message-mutate", conversation, message["id"], PORCH_TOKEN=token, PORCH_COMMAND=json.dumps(command))
    message_mutate(admin_token, op="react", emoji="🐈", active=True)
    message_mutate(user_token, op="pin", pinned=True)
    assert cli("message-pins", conversation, PORCH_TOKEN=admin_token)["messages"][0]["pinned"]
    message_mutate(user_token, op="edit", content={"format":"markdown","text":"edited text"}, mentions=[], mention_everyone=False)
    cli("message-read", conversation, message["id"], PORCH_TOKEN=admin_token)
    message_mutate(user_token, op="delete")
    assert cli("message-delta", conversation, PORCH_TOKEN=admin_token, PORCH_AFTER="0")["messages"][0]["deleted"]
    dm = cli("dm-create", PORCH_TOKEN=user_token, PORCH_COMMAND=json.dumps({"participants":[admin["user"]["id"]],"name":None}))
    dm_body = {"nonce":"smoke_direct_nonce_0001","content":{"format":"markdown","text":"direct hello"},"reply_to":None}
    cli("message-send", dm["id"], PORCH_TOKEN=user_token, PORCH_MESSAGE=json.dumps(dm_body))
    assert cli("messages", dm["id"], PORCH_TOKEN=admin_token)["messages"][0]["content"]["text"] == "direct hello"
    mutate(user_token, op="set_overwrites", channel_id=None, category_id=category,
           overwrites=[{"target":{"kind":"everyone"}, "allow":0, "deny":1}])
    snapshot = cli("community", cid, PORCH_TOKEN=admin_token)
    assert len(snapshot["channels"]) == 1 and not snapshot["categories"]
    assert "clubhouse" not in json.dumps(snapshot)
    role = mutate(user_token, op="create_role", name="Regular", permissions=1, position=1)["id"]
    mutate(user_token, op="set_overwrites", channel_id=None, category_id=category,
           overwrites=[{"target":{"kind":"everyone"},"allow":0,"deny":1},
                       {"target":{"kind":"role","id":role},"allow":1,"deny":0}])
    mutate(user_token, op="set_member_roles", user_id=admin["user"]["id"], role_ids=[role])
    assert len(cli("community", cid, PORCH_TOKEN=admin_token)["channels"]) == 2
    mutate(user_token, op="set_member_roles", user_id=admin["user"]["id"], role_ids=[])
    assert len(cli("community", cid, PORCH_TOKEN=admin_token)["channels"]) == 1
    mutate(user_token, op="ban", user_id=admin["user"]["id"])
    cli("community-join", PORCH_TOKEN=admin_token, PORCH_COMMUNITY_INVITE=ci["code"], fail=True)
    mutate(user_token, op="unban", user_id=admin["user"]["id"])
    cli("community-join", PORCH_TOKEN=admin_token, PORCH_COMMUNITY_INVITE=ci["code"])
    listed = cli("community-invites", cid, PORCH_TOKEN=user_token)
    assert listed[0]["uses"] == 2 and "code" not in json.dumps(listed)
    cli("community-invite-revoke", cid, ci["id"], PORCH_TOKEN=user_token)
    mutate(admin_token, op="leave")
    mutate(user_token, op="delete_channel", id=channel)
    mutate(user_token, op="delete_category", id=category)
    mutate(user_token, op="delete_channel", id=voice)
    mutate(user_token, op="delete_role", id=role)
    mutate(user_token, op="delete")
    cli("community", cid, PORCH_TOKEN=user_token, fail=True)
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
    print("CLI identity + community + messaging smoke passed, bootstrap/register/login/invites/session revocation/logout proven over HTTP")
finally:
    server.terminate()
    server.communicate(timeout=10)
