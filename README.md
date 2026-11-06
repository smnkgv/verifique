# verifique

A verification queue for coding agents, running on your own laptop.

## What it's for

Writing code scales to dozens of agents. Verifying it on one laptop doesn't.

Run fifteen agents and they tend to finish around the same time. Each one starts a copy of your app
to check its work, and they collide over ports, databases and memory. Worktrees separate your files,
not any of that. Moving to the cloud avoids the collisions, but gives up the fast feedback of working
locally.

verifique keeps verification on your laptop and runs only what the laptop can carry. Agents stop
waiting for each other, and an agent no longer marks its own homework.

## How it works

1. **An agent hands its work over.** It submits your original request and how to reach the change.
   Uncommitted work is captured for it.
2. **It keeps working.** It waits in the background. Repositories take turns, and fixes go ahead of
   new work.
3. **The check runs when a slot is free.** Your app runs natively, with its own ports and fresh
   databases on the servers you already have.
4. **The check ends in a verdict with proof.** A check can be any command, such as a test suite, or
   a fresh agent that compares the work with what you asked for. If it fails, the agent that
   submitted it fixes it.

```
agents, in any repo            server                       your laptop
───────────────────            ──────                       ───────────
verifique submit ───────▶ queue ─▶ a slot is free ───────────▶ fresh databases, your apps, your check
verifique wait   ◀────── verdict stored against the content ◀─ pass, fail or blocked
```

One server runs per machine and serves every project on it, because projects share the machine's
memory and ports. It knows nothing about your framework or how you test.

## Status

Version 0.1, and early. It has been tested on real web apps and on native iOS and Android apps, and it works with Claude Code today. [docs/DESIGN.md](docs/DESIGN.md) holds the decisions behind it.

## Install

On macOS or Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/smnkgv/verifique/main/install.sh | sh
```

Then start the queue:

```bash
verifique service install
```

## Prerequisites

Database servers must already be running, however you installed them. verifique never installs one, and
only ever creates and drops databases named `verifique_*`. It supports Postgres, Mongo and Redis, using
`psql`, `mongosh` and `redis-cli`. Redis is the exception to the naming: each slot owns the numbered database that matches it, and clears it for every check, so keep nothing else in those. A slot Redis has no database for is refused.

**Your `.env` is live.** Checks run your app unattended with the project's own `.env`, so anything in it
is real: a check can send real email, charge real cards or write to any service it points at. Only
slot ports and the settings in `verifique.yaml` are replaced. `verifique doctor` names remote databases it
finds, but it cannot see everything.

## Try it

[examples/shop](examples/shop) is a one-page shop with a broken Search button. A real agent fails it, you fix it, and the same check passes, in about two minutes.

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
-   `maxSlots` at the top of `verifique.yaml` limits how many slots one project takes at once.
-   Slot copies and their builds use real disk. When less than 5 GB is free, verifique deletes the copies no check is using, and if that isn't enough, checks wait. Change the limit with `verifique serve --min-free-gb`.
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

`verifique slots <n>` changes how many checks run at once while the server runs. A check that's
already running is never stopped, and restarting the service goes back to the installed number.

`verifique status` shows the queue and what needs you. `verifique show <job>` gives one job's proof, and
every command that reads state takes `--json`. Slots keep their worktrees between checks so builds
stay fast; `verifique clean` removes them.

## Status-bar panel

On macOS, a small app adds a tick to the menu bar. Click it to see what needs you, what's running,
what's queued and recent verdicts with their proof, and to change how many checks run at once. It
reads the CLI's `--json` output and acts only through the CLI, so it can't do anything the CLI can't.

The tick turns amber with the number of checks left while they run, green for five minutes after the
last one passes, and red with a count when something needs you.

```bash
panel/bundle.sh
open panel/target/verifique.app
```

The build is signed for your own machine only. Press the pin in the panel, or right-click the tick,
to keep it on screen in the top right corner, semi-transparent, on every desktop.

## Build

Rust 1.85 or later, on macOS or Linux.

```bash
cargo build --release
cargo test
```

Then put the binary on your `PATH`, for example:

```bash
ln -s "$PWD/target/release/verifique" ~/.local/bin/verifique
```

The tests need `git`, `python3` and running Postgres, Mongo and Redis. [e2e/](e2e/README.md) tests everything from the outside, from a fresh install to a real agent.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
