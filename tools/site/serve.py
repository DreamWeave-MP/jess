#!/usr/bin/env python3
"""Tiny dev-only HTTP/watch shim for the Jess-built documentation site.

The site generation itself remains entirely Jess + the dreamweave-site package. Python is used
only because Jess/L3i currently exposes DreamNet transport, not browser-facing raw TCP sockets.
"""

from __future__ import annotations

import functools
import http.server
import os
from pathlib import Path
import shutil
import socketserver
import subprocess
import sys
import threading
import time

WATCH_DIRS = ("content", "templates", "sass", "static", "data")
WATCH_FILES = ("config.toml", "zola.toml")
POLL_SECONDS = 0.35


def snapshot(site_root: Path) -> tuple[tuple[str, int, int], ...]:
    rows: list[tuple[str, int, int]] = []
    for name in WATCH_FILES:
        path = site_root / name
        if path.is_file():
            stat = path.stat()
            rows.append((name, stat.st_mtime_ns, stat.st_size))
    for dirname in WATCH_DIRS:
        root = site_root / dirname
        if not root.is_dir():
            continue
        for path in sorted(p for p in root.rglob("*") if p.is_file()):
            stat = path.stat()
            rows.append((str(path.relative_to(site_root)), stat.st_mtime_ns, stat.st_size))
    return tuple(rows)


def site_package(repo: Path) -> Path:
    configured = os.environ.get("JESS_SITE_PACKAGE")
    if configured:
        return Path(configured).expanduser().resolve()
    checkout = repo / "jpk" / "dreamweave-site"
    if (checkout / "jess.toml").is_file():
        return checkout
    artifact = repo / "tools" / "site" / "dreamweave-site.jpk"
    if artifact.is_file():
        return artifact
    raise FileNotFoundError(
        "no site-generator package found; check it out at jpk/dreamweave-site or set JESS_SITE_PACKAGE"
    )


def build(repo: Path, output: Path) -> bool:
    site_root = repo / "site"
    try:
        package = site_package(repo)
    except FileNotFoundError as error:
        print(f"[jess] {error}", file=sys.stderr)
        return False
    script = repo / "tools" / "site" / "build.luau"
    staging = output.with_name(output.name + ".dev-next")
    shutil.rmtree(staging, ignore_errors=True)

    command = [
        "jess",
        "run",
        "--package",
        str(package),
        str(script),
        str(site_root),
        str(staging),
        "/",
    ]
    result = subprocess.run(command, cwd=repo)
    if result.returncode != 0:
        shutil.rmtree(staging, ignore_errors=True)
        print("[jess] rebuild failed; serving the last successful build", file=sys.stderr)
        return False

    shutil.rmtree(output, ignore_errors=True)
    staging.rename(output)
    return True


class Handler(http.server.SimpleHTTPRequestHandler):
    def send_error(self, code: int, message: str | None = None, explain: str | None = None) -> None:
        if code == 404:
            custom = Path(self.directory) / "404.html"
            if custom.is_file():
                body = custom.read_bytes()
                self.send_response(404)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(body)
                return
        super().send_error(code, message, explain)

    def log_message(self, fmt: str, *args: object) -> None:
        sys.stderr.write("[serve] " + (fmt % args) + "\n")


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def watch(repo: Path, output: Path, stop: threading.Event) -> None:
    site_root = repo / "site"
    previous = snapshot(site_root)
    while not stop.wait(POLL_SECONDS):
        current = snapshot(site_root)
        if current == previous:
            continue
        previous = current
        print("[jess] source changed; rebuilding...")
        build(repo, output)


def main() -> int:
    if len(sys.argv) < 2:
        raise SystemExit("usage: serve.py REPOSITORY [HOST] [PORT]")

    repo = Path(sys.argv[1]).resolve()
    host = sys.argv[2] if len(sys.argv) > 2 else "127.0.0.1"
    port = int(sys.argv[3]) if len(sys.argv) > 3 else 1111
    output = repo / "site" / "public"

    if not build(repo, output):
        return 1

    handler = functools.partial(Handler, directory=str(output))
    server = Server((host, port), handler)
    stop = threading.Event()
    watcher = threading.Thread(target=watch, args=(repo, output, stop), daemon=True)
    watcher.start()

    visible_host = "127.0.0.1" if host in ("0.0.0.0", "::") else host
    print(f"[serve] http://{visible_host}:{port}/")
    print("[serve] watching site/content, templates, sass, static, data, and config")
    try:
        server.serve_forever(poll_interval=0.25)
    except KeyboardInterrupt:
        print("\n[serve] stopped")
    finally:
        stop.set()
        server.server_close()
        watcher.join(timeout=1.0)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
