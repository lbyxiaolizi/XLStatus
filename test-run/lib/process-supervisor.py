#!/usr/bin/env python3

import os
import signal
import subprocess
import sys
import time


def process_group_members(process_group_id: int) -> list[int]:
    probe = subprocess.Popen(
        ["ps", "-axo", "pid=,pgid="],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    stdout, _ = probe.communicate()
    if probe.returncode != 0:
        return [-1]

    excluded = {os.getpid(), probe.pid}
    members: list[int] = []
    for line in stdout.splitlines():
        fields = line.split()
        if len(fields) != 2:
            continue
        try:
            pid, pgid = (int(field) for field in fields)
        except ValueError:
            continue
        if pgid == process_group_id and pid not in excluded:
            members.append(pid)
    return members


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: process-supervisor.py TOKEN COMMAND [ARG ...]", file=sys.stderr)
        return 2

    os.setsid()
    # The cleanup helper signals the whole group. Keep the supervisor alive
    # long enough to reap a cooperative child; SIGKILL handles stuck groups.
    signal.signal(signal.SIGTERM, lambda _signum, _frame: None)
    child = subprocess.Popen(sys.argv[2:])
    returncode = child.wait()

    # A command can exit after spawning children that remain in this session.
    # Signal the owned group once more before the supervisor exits so those
    # descendants cannot outlive a completed or failed verification service.
    try:
        os.killpg(os.getpgrp(), signal.SIGTERM)
    except ProcessLookupError:
        pass

    deadline = time.monotonic() + 1.0
    while time.monotonic() < deadline:
        if not process_group_members(os.getpgrp()):
            return returncode
        time.sleep(0.05)

    # The session is owned exclusively by this supervisor. Killing the group
    # also terminates the supervisor, which is intentional when a descendant
    # ignored the graceful shutdown signal or group enumeration failed.
    os.killpg(os.getpgrp(), signal.SIGKILL)
    return returncode


if __name__ == "__main__":
    raise SystemExit(main())
