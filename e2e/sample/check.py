"""Fails if this job can see anything that belongs to another job."""
import json, os, random, subprocess, time, urllib.request


def run(*args):
    return subprocess.run(args, capture_output=True, text=True, check=True).stdout.strip()


def psql(sql):
    return run("psql", "-tAc", sql)


def redis(*args):
    return run("redis-cli", "-h", os.environ["REDIS_HOST"], "-p", os.environ["REDIS_PORT"], *args)


marker = open("marker.txt").read().strip()
prefix = os.environ["REDIS_PREFIX"]
problems = []
psql(f"insert into visits values ('{marker}')")
redis("set", f"{prefix}visit", marker)
time.sleep(random.uniform(3, 7))
served = urllib.request.urlopen(f"http://127.0.0.1:{os.environ['VERIFIQUE_PORT_SHOP']}/").read().decode()
if served != marker:
    problems.append(f"the app answered as {served}")
rows = psql("select string_agg(who, ',') from visits")
if rows != marker:
    problems.append(f"the database held {rows}")
keys = sorted(redis("--scan", "--pattern", f"{prefix}*").split())
if keys != [f"{prefix}seeded", f"{prefix}visit"] or redis("get", f"{prefix}visit") != marker:
    problems.append(f"redis held {keys}")
if open("behaviour.txt").read().strip() == "bad":
    problems.append("the planted bug is here")
verdict = {"verdict": "failed", "summary": "; ".join(problems)} if problems else {"verdict": "passed", "summary": f"{marker} saw only its own data"}
json.dump(verdict, open(os.environ["VERIFIQUE_RESULT"], "w"))
