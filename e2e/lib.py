"""What the end-to-end journeys share: a queue of their own, and the commands a person would type."""
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = Path(os.environ.get("VERIFIQUE_BIN", ROOT / "target/release/verifique"))
# Clear of the default slots, and below 16 because Redis has sixteen databases.
FIRST_SLOT = os.environ.get("E2E_FIRST_SLOT", "9")


def fail(message):
    sys.exit(f"e2e failed: {message}")


class Queue:
    """A real server with a home of its own, so a journey can never touch anyone's real queue."""

    def __init__(self, slots=2):
        self.slots = str(slots)
        self.dir = Path(tempfile.mkdtemp(prefix="verifique-e2e-"))
        self.home = self.dir / "home"
        self.env = {**os.environ, "VERIFIQUE_HOME": str(self.home), "PATH": f"{BINARY.parent}:{os.environ['PATH']}"}
        self.server = None

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *_):
        self.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def start(self):
        args = ["verifique", "serve", "--slots", self.slots, "--first-slot", FIRST_SLOT]
        log = open(self.dir / "server.log", "a")
        self.server = subprocess.Popen(args, env=self.env, stdout=log, stderr=subprocess.STDOUT, cwd=self.dir)
        self.wait_until(lambda: self.status()["server"], "the server to start")

    def stop(self):
        if self.server and self.server.poll() is None:
            self.server.send_signal(signal.SIGTERM)
            self.server.wait(60)

    def crash(self):
        self.server.kill()
        self.server.wait()

    def run(self, *args, cwd=None):
        return subprocess.run(["verifique", *args], env=self.env, cwd=cwd or self.dir, capture_output=True, text=True)

    def status(self):
        return json.loads(self.run("status", "--json").stdout)

    def wait_until(self, condition, what, seconds=30):
        end = time.time() + seconds
        while not condition():
            if time.time() > end:
                fail(f"timed out waiting for {what}")
            time.sleep(0.2)


def git(*args, cwd):
    identity = ["-c", "user.name=e2e", "-c", "user.email=e2e@example.test"]
    subprocess.run(["git", *identity, *args], cwd=cwd, check=True, capture_output=True)


def project(source, path):
    """A copy of a project in a repository of its own, as someone would have it."""
    shutil.copytree(source, path)
    git("init", "-q", "-b", "main", cwd=path)
    git("add", "-A", cwd=path)
    git("commit", "-qm", "The project", cwd=path)
    return path


def readme_commands(path):
    """Every `bash` block in a README, in order, so a journey runs what the docs really say."""
    return re.findall(r"```bash\n(.*?)```", Path(path).read_text(), re.S)
