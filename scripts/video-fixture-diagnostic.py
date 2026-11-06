"""Bounded, trusted-fixture diagnostic for distro FFmpeg compatibility.

This does not process user input or change production decoder limits. Keep the
arguments synchronized with previews.rs; the real HTTP/native tests remain gates.
"""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
fixture = root / "crates/porch-server/tests/fixtures/poster.mp4"
args = [
    "/usr/bin/prlimit", "--as=536870912", "--cpu=4", "--nofile=32",
    "--fsize=1048576", "--", "/usr/bin/ffmpeg",
    "-hide_banner", "-loglevel", "error", "-nostdin", "-max_alloc", "33554432",
    "-threads", "1", "-filter_threads", "1", "-protocol_whitelist", "pipe",
    "-format_whitelist", "mov,matroska,webm",
    "-codec_whitelist", "h264,hevc,vp8,vp9,av1", "-max_pixels", "8388608",
    "-probesize", "1048576", "-analyzeduration", "1000000", "-i", "pipe:0",
    "-map", "0:v:0", "-an", "-sn", "-dn", "-frames:v", "1",
    "-vf", "scale=480:320:force_original_aspect_ratio=decrease",
    "-c:v", "png", "-threads", "1", "-f", "image2pipe", "pipe:1",
]
env = {key: "1" for key in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS")}
with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
    result = subprocess.run(args, input=fixture.read_bytes(), stdout=out, stderr=err, env=env, timeout=5)
    size = out.tell()
    out.seek(0)
    signature = out.read(8)
    err.seek(0)
    detail = err.read(8192).decode("utf-8", errors="replace")
    print(f"Trusted FFmpeg fixture: status={result.returncode}, output_bytes={size}")
    print(detail or "(no stderr)", flush=True)
    assert result.returncode == 0 and signature == b"\x89PNG\r\n\x1a\n", "bounded fixture decode failed"
