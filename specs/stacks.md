---
status: active
last-reviewed: 2026-11-09
owner: Hlib Semeniuk
confidence: best-effort
related: [slots-and-isolation, checks, databases, queue, setup]
glossary_terms: [Stack, Started stack, App, Base port, Ready path, Prepare, Seed, Extra seed, Setup step, Slot, Job, .env]
---

# Stacks and started stacks

A stack tells verifique what a check needs running: how to build the code, how to fill its
databases, which settings it gets, and which apps to start. The same stack can be started by hand,
in a slot of its own for a while, so an agent or a person can investigate without starting the app
themselves.

## Why

verifique knows nothing about a project's framework or how it tests, so the project describes its
running app once, in `verifique.yaml`, and verifique does the rest in every slot.

Apps run as production builds rather than dev servers: they start faster and use far less memory
across several copies.

An agent reproducing a bug used to start the app itself, and so collided with every other agent on
the machine. A started stack goes through the same queue with the same isolation, so investigating
is as safe as checking. It stops by itself, so a forgotten stack or a crashed agent cannot jam the
queue, and it never takes the last slot, so checks keep moving.

People sometimes need real data to investigate. A started stack can load it. Checks never do, so
their verdicts stay reproducible.

## Behaviour

### What a stack is

1. A stack is named under `stacks` in `verifique.yaml`. Every part is optional: `prepare` (a list of
   steps), `seed`, `seeds` (named extra seeds), `setup`, `env`, `envFiles`, `needs` and `apps`.
2. Each app has `run`, and may have `cwd` (a folder within the repository), `port` (its base port),
   `ready` (its ready path) and `env`.
3. An app with a ready path must have a port to check it on.
4. A stack that uses a database server's values must declare that server under `services` (see
   [databases](databases.md)).
5. A stack needs no apps. One with only `prepare`, `seed` and `env` gives a check its own databases
   without building or starting the app, such as for a test suite.
6. A stack can name things it needs under `needs`, such as a simulator. Every check on that stack
   needs them too (see [checks](checks.md)).

### How a stack comes up

7. In every slot, in this order: the stack's `prepare` steps and then the check's own; the slot's
   databases are created fresh and the `seed` runs on them; every app starts; verifique waits until
   every ready path answers; the `setup` steps run, the stack's and then the check's. Only then does
   the check run.
8. An app is ready when a request to its ready path on its port gets a successful answer. If an app
   exits before it is ready, or is not ready in time, the job ends in an error.
9. The ready path must prove the app works, not only that a port is open. A ready check that passes
   on a broken app makes every verdict on that stack worth less.
10. What `setup` prints reaches the checker and is kept with the job. It must only ever carry
    throwaway credentials (see [safety](safety.md)).
11. Each step has a time limit (see [checks](checks.md)).

### Starting a stack by hand

12. `verifique stack start [<stack>] [--minutes <n>]` queues for a slot like a check does, starts the
    stack there, and returns once it is up. When the project has more than one stack, the stack
    must be named. It runs for 30 minutes unless told otherwise.
13. Once the stack is up, it prints each app's address, what `setup` printed, where the logs are, and
    how to run a command in it and stop it. `--json` prints the same as JSON.
14. If the stack ends before it comes up, `stack start` prints why and exits 2.
15. A started stack has the code as it was when it started, committed or not. To see later changes,
    stop it and start a new one.
16. It gives no verdict, and never affects needs-you.
17. It stops when its time runs out, when `verifique stack stop <job>` or `verifique cancel <job>` is
    run, when one of its apps exits, or when the queue stops or restarts.
18. A started stack takes a slot as a check does. The slot count is how many copies run at once,
    started stacks and checks alike, and no slot is kept back for either.
19. `stack stop` on a check is refused, and points to `verifique cancel`.

### Real data for a started stack

20. `verifique stack start --seed <name> -- <files>` runs one of the stack's `seeds` after its usual
    seed, with the files added to the end of its command, such as to load a real dump into the
    slot's own databases.
21. Naming a seed the stack doesn't have lists the ones it has. Anything after `--` without `--seed`
    is refused.
22. Checks never run extra seeds.

### Running a command in a started stack

23. `verifique stack exec <job> -- <command>` runs a command in the started stack's slot copy, with
    the stack's settings and the queue's `PATH`, as its seed runs, but once the apps are up: such as
    one of the project's own tools that loads data through them. Its exit code is passed on.
24. The command is joined and run by the shell, as ssh does, so whatever the user's own shell should
    not read, such as `&&`, must be quoted.
25. `stack exec` is refused before the stack is up, after it has stopped, and on a check.
26. The settings `stack exec` uses include the values from `.env`, so they are kept readable only by
    the user, and removed when the stack stops.

## Never

-   A started stack never gives a verdict.
-   Checks never use extra seeds.

## Limits

-   With every slot taken by started stacks, checks wait until one stops or its time runs out.

-   A started stack does not follow the work: it has the code from when it started.
-   A started stack ends when the queue restarts, and whoever was using it loses it.

## Open questions

