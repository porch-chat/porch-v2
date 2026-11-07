#!/usr/bin/env python3
"""Test-only stand-in for LiveKit's Twirp RoomService (no media).
Answers every RPC with an empty protobuf message so porch-server's REAL ticket,
lease and flag logic runs locally. It carries no audio; nothing here verifies sound."""
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("content-length", 0)); self.rfile.read(n)
        self.send_response(200)
        self.send_header("content-type", "application/protobuf")
        self.send_header("content-length", "0"); self.end_headers()
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
