#!/usr/bin/env python3
"""A busy day in minutes: 15 sessions across 3 repositories, while the queue is killed and resized.

Every job has a known right answer, and each check fails if it can see another job's app, database or
Redis keys. A key left in Redis database 0 has to survive, because no slot may reach it.
"""
import json
import subprocess
import sys
import threading
import time

from lib import FIRST_SLOT, ROOT, Queue, fail, git, project

SAMPLE = ROOT / "e2e/sample"
CANARY = f"verifique-e2e-canary-{int(time.time())}"
BASE_PORT = 4600

# good and bad are what the change should get. replaced is replaced by the session's next round.
PLAN = {
    "a": [["good"], ["good"], ["bad", "good"], ["good", "good"], ["bad", "good"]],
    "b": [["good"], ["replaced", "good"], ["good", "good"], ["bad", "good"], ["good"]],
    "c": [["bad", "good"], ["good"], ["good", "good"], ["good"], ["replaced", "good"]],
}
EXIT_CODES = {"good": 0, "bad": 1}


class Results:
    def __init__(self):
        self.rows = []
        self.lock = threading.Lock()

    def add(self, session, kind, job, code, wanted, text):
        with self.lock:
            right = code == wanted
            self.rows.append(right)
            print(f"{'ok ' if right else 'BAD'} {session} {kind} job {job}: {text[:90]}", flush=True)


def session(queue, repo, name, rounds, results):
    tree = queue.dir / "trees" / name
    git("worktree", "add", "-q", "-b", name, str(tree), cwd=repo)
    replaced = None
    for number, kind in enumerate(rounds, 1):
        (tree / "marker.txt").write_text(f"{name}-{number}\n")
        (tree / "behaviour.txt").write_text("bad\n" if kind == "bad" else "good\n")
        git("add", "-A", cwd=tree)
        git("commit", "-qm", f"Round {number}", cwd=tree)
        out = queue.run("submit", "--check", "isolation", cwd=tree)
        if out.returncode != 0:
            results.add(name, kind, None, -1, 0, out.stdout + out.stderr)
            return
        job = out.stdout.strip().split()[-1]
        if kind == "replaced":
            replaced = job
            continue
        waited = queue.run("wait", job, cwd=tree)
        results.add(name, kind, job, waited.returncode, EXIT_CODES[kind], waited.stdout.strip())
    if replaced:
        status = json.loads(queue.run("show", replaced, "--json").stdout)["job"]["status"]
        results.add(name, "replaced", replaced, status, "superseded", status)


def redis(*args):
    return subprocess.run(["redis-cli", *args], capture_output=True, text=True, check=True).stdout.strip()


def listening():
    ports = range(BASE_PORT + int(FIRST_SLOT), BASE_PORT + int(FIRST_SLOT) + 3)
    out = subprocess.run(["lsof", "-nP", "-iTCP", "-sTCP:LISTEN"], capture_output=True, text=True).stdout
    return [line for line in out.splitlines() if any(f":{port} " in line for port in ports)]


def main():
    redis("set", CANARY, "alive")
    results = Results()
    try:
        with Queue(slots=2) as queue:
            sessions = []
            for letter, rounds in PLAN.items():
                repo = project(SAMPLE, queue.dir / f"e2e-orders-{letter}")
                for number, plan in enumerate(rounds, 1):
                    sessions.append((repo, f"s-{letter}{number}", plan))
            threads = [threading.Thread(target=session, args=(queue, *item, results)) for item in sessions]
            for thread in threads:
                thread.start()

            queue.wait_until(lambda: queue.status()["running"], "a check to start", 60)
            time.sleep(3)
            running = [job["id"] for job in queue.status()["running"]]
            queue.crash()
            print(f"killed the server with jobs {running} running", flush=True)
            time.sleep(16)
            queue.start()
            print("the server is back", flush=True)
            queue.run("slots", "3")
            time.sleep(10)
            queue.run("slots", "1")
            time.sleep(10)
            queue.run("slots", "2")

            for thread in threads:
                thread.join()
            queue.wait_until(lambda: not queue.status()["running"], "the last check to end")
        left = listening()
        if left:
            fail(f"left listening: {left}")
    finally:
        canary = redis("get", CANARY)
        redis("del", CANARY)
    if canary != "alive":
        fail("a check wiped Redis database 0")
    wrong = results.rows.count(False)
    print(f"{len(sessions)} sessions, {len(results.rows)} verdicts, {wrong} wrong")
    sys.exit(1 if wrong else 0)


main()
