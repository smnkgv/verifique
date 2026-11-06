#!/usr/bin/env python3
"""How well a real agent judges a project: each case is a change with a known right answer.

    python3 e2e/scoreboard.py --repo <a project> --cases <a folder of cases> [--check manual]

A case is a folder holding change.patch, request.md, brief.txt and expected.json, which says
{"expected": "failed"} for a planted bug or {"expected": "passed"} for a good change. The bar is every
good change passed and at least four in five planted bugs caught. Private projects keep their cases
outside this repository. It uses the queue that is already running, or VERIFIQUE_HOME's.
"""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

CODES = {0: "passed", 1: "failed", 2: "blocked", 3: "replaced"}
BAR = 0.8


def run(*args, cwd):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--check", default="manual")
    options = parser.parse_args()
    repo, rows = options.repo.resolve(), []
    if run("git", "status", "--porcelain", cwd=repo).stdout.strip():
        sys.exit(f"{repo} has uncommitted changes, and each case needs a clean start")
    branch = run("git", "branch", "--show-current", cwd=repo).stdout.strip()
    run("git", "checkout", "-q", "--detach", cwd=repo)
    try:
        for case in sorted(path for path in options.cases.resolve().iterdir() if path.is_dir()):
            patch = str(case / "change.patch")
            if run("git", "apply", patch, cwd=repo).returncode != 0:
                rows.append((case.name, "?", "could not apply", ""))
                continue
            wanted = json.loads((case / "expected.json").read_text())["expected"]
            submitted = run("verifique", "submit", "--check", options.check,
                            "--request-file", str(case / "request.md"),
                            "--brief", (case / "brief.txt").read_text().strip(), cwd=repo)
            job = submitted.stdout.strip().split()[-1]
            waited = run("verifique", "wait", job, cwd=repo)
            run("git", "apply", "-R", patch, cwd=repo)
            rows.append((case.name, wanted, CODES.get(waited.returncode, "error"), waited.stdout.strip()[:120]))
            print(f"{case.name}: wanted {wanted}, got {rows[-1][2]}", flush=True)
    finally:
        run("git", "checkout", "-q", branch or "-", cwd=repo)
    bugs = [row for row in rows if row[1] == "failed"]
    caught = sum(row[2] == "failed" for row in bugs)
    good = [row for row in rows if row[1] == "passed"]
    wrong = sum(row[2] != "passed" for row in good)
    print(f"{caught} of {len(bugs)} planted bugs caught, {len(good) - wrong} of {len(good)} good changes passed")
    sys.exit(0 if wrong == 0 and bugs and caught / len(bugs) >= BAR else 1)


main()
