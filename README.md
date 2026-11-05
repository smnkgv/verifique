# verifique

Run swarms of coding agents across several repositories on one laptop, and verify their work
locally without verifications colliding. Agents hand over what they want checked and carry on; the
laptop runs each check when a slot is free, and every check ends in a verdict with proof.

Early, and not released yet: it is being proven on real projects first, so for now you build it from
source. [docs/DESIGN.md](docs/DESIGN.md) holds the decisions behind it.

## How it works

One server runs per machine and serves every project on it, because projects share the machine's
memory and ports. Apps run as normal processes on the laptop, several copies at once, each on its own
slot with its own ports and its own databases.

```
agents, in any repo            server                       your laptop
───────────────────            ──────                       ───────────
verifique submit ───────▶ queue ─▶ a slot is free ───────────▶ fresh databases, your apps, your check
verifique wait   ◀────── verdict stored against the content ◀─ pass, fail or blocked
```

It knows nothing about your framework or how you test. Each check starts on clean data: its databases
are created, your `seed` runs, and they are dropped afterwards.

## Prerequisites

Database servers must already be running, however you installed them. verifique never installs one, and
only ever creates and drops databases named `verifique_*`. It supports Postgres, Mongo and Redis, using
`psql`, `mongosh` and `redis-cli`.

**Your `.env` is live.** Checks run your app unattended with the project's own `.env`, so anything in it
is real: a check can send real email, charge real cards or write to any service it points at. Only
slot ports and the settings in `verifique.yaml` are replaced. `verifique doctor` names remote databases it
finds, but it cannot see everything.

## Connect a project

Run this and give the output to your coding agent:

```bash
verifique init
```

It writes one file at the repository root, then proves it with `verifique doctor`, which starts two
copies side by side and confirms each used its own database:

```yaml
services:
    postgres: { url: postgres://postgres:postgres@127.0.0.1:5432 }
    redis: {}

stacks:
    web:
        prepare:
            - pnpm install --frozen-lockfile
            - pnpm build
        seed: pnpm db:migrate && pnpm db:seed # on a fresh database, for every check
        setup: ./scripts/sign-in.sh # optional: prints sign-in details for a throwaway user
        env: # wins over .env
            DATABASE_URL: "{postgres.url}"
            REDIS_URL: "{redis.url}"
        apps:
            web:
                run: pnpm start
                port: 3500 # base + slot
                ready: /
                env:
                    PORT: "{port}"

checks:
    typecheck:
        run: pnpm typecheck
    e2e:
        stack: web
        slots: 2 # one stack per worker, from one build
        run: pnpm playwright test
    manual:
        type: agent
        stack: web
        agent: claude # or { command: "your-agent $VERIFIQUE_PROMPT" }
        tools: [] # added to your own agent settings
        notes: |
            What a newcomer would need to know.
```

A project with its own isolation can give a stack a `start` command that takes `{slot}` instead of
`apps`, and declare its own `ports` and `ready` URLs.

Commands get `{slot}`, `{port}`, `{port.<app>}`, `{app.<app>.url}`, and for each service `.url`,
`.server`, `.database`, `.host`, `.port`, `.user` and `.password`. Redis adds `.db` and `.prefix`.

A check also gets these variables: `VERIFIQUE_SLOT`, `VERIFIQUE_SLOTS`, `VERIFIQUE_STACKS` (a JSON file with
every stack's ports and values), `VERIFIQUE_PORT_<APP>`, `VERIFIQUE_COMMIT`, `VERIFIQUE_TREE`,
`VERIFIQUE_REQUEST`, `VERIFIQUE_BRIEF`, `VERIFIQUE_DIFF`, `VERIFIQUE_SETUP`, `VERIFIQUE_PROMPT`, `VERIFIQUE_RESULT`,
`VERIFIQUE_EVIDENCE`, `VERIFIQUE_REPO` and `VERIFIQUE_JOB`.

How a check reports:

-   By default, exit 0 passes and anything else fails.
-   `verdict: file` means it writes `{"verdict": "passed" | "failed" | "blocked", "summary": "…"}` to
    `$VERIFIQUE_RESULT`. A crash, or no readable file, is an error and never a pass.
-   `type: agent` also needs `steps`, `saw` and at least one file in `$VERIFIQUE_EVIDENCE` before a pass
    counts.

Anything limited, such as a simulator, goes in `needs` and is declared on the machine with
`--resource simulator=1`.

## Use it

Start the server once, on the machine:

```bash
verifique service install --slots 2 --resource simulator=1
```

After a new build, run `verifique service restart`. Then, from any repository or worktree, an agent of any kind can run:

```bash
verifique submit --check manual --request-file request.md --brief "Open Settings, then Filters"
verifique wait <job>     # exit 0 passed, 1 failed, 2 blocked or error, 3 superseded
```

-   Uncommitted work is captured without touching the branch, and verdicts belong to the content, so
    a later commit of the same content already counts as verified.
-   A newer submission replaces that branch's queued run of the same check.
-   Repositories take turns for free slots, and a retry goes ahead of new work. `verifique next <job>`
    overrides that.
-   A check with `slots` gets as many stacks as the machine has, and smaller jobs wait so it cannot
    starve.
-   Three failures in a row, or one `blocked`, put a branch in `needs-you` and raise a desktop
    notification. Submissions are then refused until `verifique clear`.

To investigate rather than verify, an agent takes a timed hold on a slot:

```bash
verifique hold --minutes 30   # waits for a slot, then prints addresses and logs
verifique release <job>       # or it ends by itself
```

A hold goes through the same queue with the same isolation, and gives no verdict. One slot always
stays free of holds, and it runs the code as it was when taken, so take a new one to see changes.

A check runs its browser headless unless it sets `browser: headed`, and `verifique submit --headed`
opens a window for one run. Commands that manage their own browser get `VERIFIQUE_HEADED=1`.

`verifique status` shows the queue and what needs you. `verifique show <job>` gives one job's proof, and
every command that reads state takes `--json`. Slots keep their worktrees between checks so builds
stay fast; `verifique clean` removes them.

## Build

Rust 1.85 or later, on macOS or Linux.

```bash
cargo build --release
cargo test
```

The tests need `git`, `python3` and running Postgres, Mongo and Redis.
