# verifique.yaml

One file at the repository root tells verifique how to start your app and which checks to run.
`verifique init` prints a guide for your coding agent, which writes it for you, and
`verifique doctor` checks it by starting two copies side by side. This page lists every setting.

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
        seeds: # optional: extra data for a stack started by hand
            dump: ./scripts/load-dump.sh
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

Commands are POSIX `sh` on every OS: the system `sh` on macOS and Linux, and Git for Windows' `sh` on
Windows. Write paths in them with forward slashes, which all three understand.

Commands get `{slot}`, `{port}`, `{port.<app>}`, `{app.<app>.url}`, and for each service `.url`,
`.server`, `.database`, `.host`, `.port`, `.user` and `.password`. Redis adds `.db` and `.prefix`.

A stack needs no apps: one with only `prepare`, `seed` and `env` gives a test suite its own
databases without building the app.

verifique reaches apps at `127.0.0.1`. `host: localhost` on a stack makes every address it gives out
use that name instead, in `{app.<app>.url}`, ready checks, the checker's prompt and what
`stack start` prints, for an app whose cookies or allowed origins only accept one name.

A check, and a command run in a started stack with `stack exec`, also gets these variables:
`VERIFIQUE_SLOT`, `VERIFIQUE_SLOTS`, `VERIFIQUE_STACKS` (a JSON file with every stack's host, ports
and values), `VERIFIQUE_PORT_<APP>`, `VERIFIQUE_COMMIT`, `VERIFIQUE_TREE`,
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
