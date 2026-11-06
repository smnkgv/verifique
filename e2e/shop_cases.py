#!/usr/bin/env python3
"""The scoreboard on the shop, once its Search button works: two planted bugs and one good change."""
import subprocess
import sys

from lib import ROOT, Queue, git, project

with Queue() as queue:
    shop = project(ROOT / "examples/shop", queue.dir / "verifique-shop")
    git("apply", "fix.patch", cwd=shop)
    git("add", "-A", cwd=shop)
    git("commit", "-qm", "Fix the Search button", cwd=shop)
    done = subprocess.run([sys.executable, str(ROOT / "e2e/scoreboard.py"), "--repo", str(shop), "--cases", str(ROOT / "e2e/cases/shop")], env=queue.env)
sys.exit(done.returncode)
