"""Stands at `derive.anthropic.tool.program` in a build: sends cache.py what bilbo started it with, and ends as the tool behind the
cache ended (../README.md). Usage: tool.py <the cache's URL> <the tool's arguments>, the tool's standard input on its own."""

import http.client
import json
import os
import sys
from pathlib import Path
from urllib.parse import urlsplit

LEFT_OUT = ("HOME", "USER", "PATH", "TMPDIR")
NO_ANSWER = 70


def started_with(block=Path("/proc/self/environ")):
    # Python adds LC_CTYPE to its own environment where the locale is C, so the kernel is asked where it keeps the block.
    try:
        block = block.read_bytes()
    except OSError:
        return dict(os.environ)
    return dict(item.decode().split("=", 1) for item in block.split(b"\0") if b"=" in item)


def request(arguments, given, stdin, folder):
    """The body, which names the request, and the header of what a machine alone decides."""
    body = {"argv": arguments, "env": {k: v for k, v in given.items() if k not in LEFT_OUT}, "stdin": stdin}
    run = {"folder": folder, "env": {k: v for k, v in given.items() if k in LEFT_OUT}}
    return json.dumps(body, sort_keys=True).encode(), json.dumps(run, sort_keys=True)


def ask(url, body, run):
    where = urlsplit(url)
    connection = http.client.HTTPConnection(where.hostname, where.port)
    try:
        connection.request("POST", where.path or "/", body=body, headers={"content-type": "application/json", "x-run": run})
        return connection.getresponse().read()
    finally:
        connection.close()


def main():
    body, run = request(sys.argv[2:], started_with(), sys.stdin.buffer.read().decode(), os.getcwd())
    try:
        ended = json.loads(ask(sys.argv[1], body, run))
        code, out = ended["exit"], ended["stdout"]
    except (OSError, http.client.HTTPException, ValueError, KeyError, TypeError):
        sys.exit(NO_ANSWER)
    sys.stdout.buffer.write(out.encode(errors="surrogateescape"))
    sys.stdout.flush()
    sys.exit(code if 0 <= code < 256 else 1)


if __name__ == "__main__":
    main()
