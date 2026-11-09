---
status: active
last-reviewed: 2026-11-09
owner: Hlib Semeniuk
confidence: best-effort
related: [databases, stacks, queue, safety, checks]
glossary_terms: [Slot, Slot number, Base port, Stack, App, Content, .env, Check, Job]
---

# Slots and isolation

Every check runs in a slot: its own copy of the repository, its own ports and its own databases. Two
checks running at the same time never test each other's code, answer on each other's ports or touch
each other's data.

## Why

With several agents at once, each check needs two things to itself. Its own copy of the code, so
agents don't change each other's files. And its own running copy of the app, on its own ports and
with its own databases, so one agent never tests another's change or trips over its data. Worktrees
alone separate the files, not the ports, databases or memory. Separate ports are easy to come by.
A fresh set of databases for every check, cleaned up afterwards, is the part people rarely build,
and verifique builds it in.

The app runs natively on the user's machine, not in a container. Builds reuse their caches between
checks, containers cost memory on a laptop, and phone simulators cannot run in one.

## Behaviour

### Slots

1. The machine has a fixed number of slots, two by default (see [queue](queue.md) for changing it).
2. Each slot has a slot number, and everything a slot owns is derived from it. Slot numbers start at
   4 by default and count up, so the default two slots are 4 and 5. `--first-slot` on
   `verifique service install` changes where they start.
3. A slot runs one job at a time.

### Its own copy of the code

4. Each slot keeps its own copy of every repository it has run, and moves that copy to the content
   being checked. The copy holds exactly the submitted files. Dependencies and build output that git
   ignores are kept from one check to the next, so builds stay incremental.
5. Every worktree of a repository shares the same slot copies: verifique knows a repository by its
   main checkout.
6. For a check with a stack, the project's `.env`, and any other files named under `envFiles`, are
   copied into the slot from the repository's main checkout, even for work submitted from a
   worktree.
7. A check without a stack runs in its slot's clean copy of the code, with no apps, no databases and
   no copy of `.env`.
8. `verifique clean` removes the slot copies no running job is using, the copies of repositories
   that no longer exist, and snapshots no queued or running job needs. A check rebuilds whatever is
   missing, so only time is lost.

### Its own ports

9. Each app runs on its base port plus the slot number. An app with `port: 3500` runs on 3504 in slot
   4 and on 3505 in slot 5.
10. Before a stack starts, every port it will use must be free. A process left listening by an
    earlier job in the same slot copy is stopped. Anything else holding one of those ports ends the
    job with an error naming the process, its id and its folder, because otherwise the check would
    run against that process instead of the code it came to verify.
11. A stack in which two apps, or two copies of one job, would use the same port is refused before
    anything starts, rather than letting one copy stop the other's app and pass anyway.
12. Commands get the slot's ports as `{port}` (the app's own), `{port.<app>}`, `{app.<app>.url}` and
    `{app.<app>.port}`, and as the variable `VERIFIQUE_PORT_<APP>`.

### Its own databases

13. Each slot gets its own databases on the user's database servers, created fresh for every check
    and dropped afterwards. [Databases](databases.md) owns how.

### Its own settings

14. Every step and app runs with the slot's values and the settings `verifique.yaml` gives it, and
    those always win over `.env`. Only what `verifique.yaml` and the slot set is replaced: anything
    else in `.env` reaches the app unchanged (see [safety](safety.md)).
15. `.env` is read the way common dotenv loaders read it (comments, `export`, quotes) and reaches
    every step and app through the environment, so a production start command needs no loader of
    its own. A line it cannot read is skipped.
16. Commands get `{slot}` and, for each database server, the values listed in
    [databases](databases.md).

### Starting clean, ending clean

17. Each check starts clean: fresh databases, the project's seed, and the code exactly as submitted.
    Nothing from an earlier check survives except ignored build output.
18. When a step finishes, anything it left running is stopped. When a job ends, its apps and
    everything they started are stopped, and its databases are dropped.

### Several slots for one check

19. A check with `slots: <n>` runs against up to that many copies of its stack, one per worker of a
    suite, each in its own slot with its own ports and databases, all from one build. It gets at
    most as many as the machine has.
20. Such a check gets `VERIFIQUE_SLOTS`, listing the slot numbers, and `VERIFIQUE_STACKS`, describing
    every copy's ports and values.
21. A check needs a stack to use more than one slot.

## Never

-   verifique never runs the project's app in a container.
-   It never stops a process by name or pattern. It stops only what it started, and processes an
    earlier job left in the same slot copy.

## Limits

-   Databases are separate per slot but share one server, so one check's load can slow another.
-   A server a checker starts by hand can outlive its job, using memory until the next job in that
    slot stops it.
-   verifique isolates ports and the database servers it is told about. Other shared things, such as
    cache and queue names, uploads, temporary folders and scheduled jobs, are isolated only if the
    project points their settings at slot values. The setup guide asks the setup agent to find them.

## Open questions

-   **Should users know slots start at 4?** The user guide's example says an app runs on "base +
    slot" but never says which slot numbers exist, so a reader expects port 3501. The first slot
    went from 4 to 1 and back to 4 in recent releases, the second time to keep checks off Redis
    database 1 and off the base port plus one. `--first-slot` has no help text. Settled by deciding
    whether the first slot is part of the interface and documenting it.
-   **Should a check without a stack get `.env`?** It gets no copy of `.env` and no databases. The
    user guide's warning that "checks run your app unattended with the `.env`" reads as if it
    applied to every check. Settled by the owner confirming the behaviour and the guide saying it.
