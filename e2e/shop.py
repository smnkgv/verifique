#!/usr/bin/env python3
"""A newcomer trying the shop: the example's README, block by block, with a real agent checking.

It fails on the broken Search button, then passes once the README's fix is applied. The only change to
the README's commands is where the copy goes, and which job the `show` step reads.
"""
import re
import subprocess

from lib import ROOT, Queue, fail, readme_commands

WANTED_EXIT = {"verifique wait": [1, 0]}


def main():
    with Queue() as queue:
        copy = queue.dir / "verifique-shop"
        cwd = ROOT
        waits = iter(WANTED_EXIT["verifique wait"])
        last_job = None
        for block in readme_commands(ROOT / "examples/shop/README.md"):
            block = block.strip().replace("~/verifique-shop", str(copy))
            if "<job>" in block:
                block = block.replace("<job>", str(last_job))
            print(f"$ {block[:100]}", flush=True)
            if block.startswith("cd "):
                cwd = copy
                block = block.split("&&", 1)[1].strip()
            if block.startswith("git commit"):
                block = block.replace("git commit", "git -c user.name=e2e -c user.email=e2e@example.test commit")
            block = block.replace("&& git commit", "&& git -c user.name=e2e -c user.email=e2e@example.test commit")
            done = subprocess.run(block, shell=True, cwd=cwd, env=queue.env, capture_output=True, text=True)
            print(done.stdout[-400:].strip(), flush=True)
            if block.startswith("verifique wait"):
                wanted = next(waits)
                if done.returncode != wanted:
                    fail(f"the check exited {done.returncode}, wanted {wanted}: {done.stdout}{done.stderr}")
                last_job = re.search(r"^\d+", queue.run("list", cwd=copy).stdout, re.M).group()
            elif done.returncode != 0:
                fail(f"{block}: {done.stderr}")
        if next(waits, None) is not None:
            fail("the README ran fewer checks than this journey expects")
    print("shop journey passed")


main()
