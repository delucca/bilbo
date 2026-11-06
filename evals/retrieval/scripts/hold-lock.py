"""Hold an exclusive lock on a file for as long as the process that started this one lives.
Usage: hold-lock.py <lockfile> <status-file> <parent-pid> <owner>
It writes `held` to the status file once it has the lock, and records <owner> in the lock file; when another process
holds the lock it writes `busy` and that process's owner instead, and exits 1. The lock goes with the process, so a
parent killed outright (SIGKILL, a reboot) frees it within a fraction of a second."""

import fcntl
import os
import sys
import time

lock, status, parent, owner = sys.argv[1:5]


def say(text):
    with open(status + ".part", "w") as f:
        f.write(text)
    os.replace(status + ".part", status)


f = open(lock, "a+")
try:
    fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    f.seek(0)
    say("busy " + f.read().strip())
    sys.exit(1)
f.seek(0)
f.truncate()
f.write(owner)
f.flush()
say("held")
while os.getppid() == int(parent):
    time.sleep(0.2)
