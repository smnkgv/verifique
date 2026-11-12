---
status: active
last-reviewed: 2026-11-12
owner: Hlib Semeniuk
confidence: best-effort
related: [stacks, checks, databases, queue, safety, slots-and-isolation, platforms, add-ons]
glossary_terms: [Doctor, Rules, Stack, Check, Slot, Database server, .env, Ready path, Setup step]
---

# Setup: install, `init` and `doctor`

Connecting a project takes the user's own coding agent. `verifique init` prints a guide, the agent
writes one file, `verifique.yaml`, `verifique doctor` proves it, and the agent proposes rules for the
project's agent instructions. After that, any agent working in the repository can hand its work
over.

## Why

verifique works with any project, from nothing, and knows nothing about frameworks or test tools.
The user's coding agent can read the code and find what two copies of the app would fight over, so
verifique gives it a guide instead of trying to guess. The guide keeps the agent honest: small edits
only, on a branch, and stop on anything bigger.

A file that looks right proves nothing. `doctor` starts two copies side by side and checks that each
really used its own databases, which catches the most common mistake: an app that ignores its
settings and still goes to the database in `.env`.

verifique proposes the rules for the project's agents and never writes them. Some projects require a
person to approve changes to their agent instructions, and nothing should force an agent to use the
queue behind the user's back.

## Behaviour

### Installing

1. The core is installed with each system's own package manager (see [platforms](platforms.md)):
   Homebrew on macOS, verifique's signed apt, dnf or pacman repository on Linux, Scoop on Windows,
   or an install script on any of them. It can also be built from source.
2. Each kind of database a project uses is then added with `verifique plugin add` (see
   [add-ons](add-ons.md)). The installers say so as the next step.
3. `verifique service install` then starts the queue in the background, and from then on at every
   logon (see [queue](queue.md)).
4. Installing the service prints a warning that `.env` is live (see [safety](safety.md)) and shows
   which common tools the service can find.

### `init`

5. `verifique init` prints a guide for the user's coding agent, and writes nothing itself.
6. The guide has the agent, in order:
    1. Check that the database servers are already running. If one is missing, tell the user and
       stop: neither verifique nor the agent installs one. If the add-on for one is not installed,
       ask the user to run `verifique plugin add <name>`.
    2. List everything two copies of the app would collide over (ports, databases, cache and queue
       names, uploads, temporary folders, scheduled jobs) and the exact settings the app reads for
       each.
    3. Write `verifique.yaml`, keeping it short. Run the app as a production build, with the exact
       start command production uses. Point every setting that names a port, a database or a shared
       resource at the slot's values. Set the time zone and anything else production sets that
       changes behaviour. Point cloud credentials in the home folder at nothing unless a check needs
       them. If `.env` lacks something, tell the user what to add rather than filling it in.
       Make sure the queue's `PATH`, which `verifique service env` shows, gives the versions
       production uses. If the app only works at one host name, such as `localhost`, name it on the
       stack. Keep every step from changing anything outside its slot, such as installers that
       clean shared caches. For a phone app, run the backend as a stack and the app on a simulator,
       declared as a limited resource.
    4. Only if the app hardcodes a value, make small edits so it reads the value from the
       environment, keeping today's value as the default, on a branch shown to the user. Anything
       larger is for the user to decide.
    5. Prove it with `verifique doctor`, never weakening a ready path to make it pass, and report
       any remote-database warning to the user. Then submit every check once and read each verdict.
       A test suite runs whole. Setup is finished only when every check has passed, or its only
       failures also happen without verifique. A failure is studied in a started stack, never by
       resubmitting the check, because every failed check counts toward needs-you.
    6. Propose the rules to the user as a change to the project's agent instructions, written the
       way those instructions write their own rules and placed where they say such rules belong.
       Change every existing rule the setup makes untrue, such as how to start the app or run its
       tests, and merge each rule with any the project keeps on the same subject, such as another
       way it verifies work, so that no two contradict. Before removing or narrowing a rule, find
       everything that still depends on it.
7. The guide tells the agent never to stop a process by name or pattern, and to stop only what it
   started itself.
8. `verifique init --rules` prints only the rules.
9. The rules tell a project's agents:
    -   Never start the app yourself, even to investigate: use `verifique stack start`, run project
        commands in it with `verifique stack exec`, see later changes in it with
        `verifique stack rebuild`, with `--fresh` after a change to the databases' structure, and
        stop it with `verifique stack stop` as soon as you are done.
    -   Never stop processes by name or pattern.
    -   Submit with the user's original request, word for word, and a brief on how to reach the
        change. Anything after `--` goes on the end of the check's command.
    -   Run `verifique wait` in the background and carry on, or end the turn. There is no need to
        commit first.
    -   On a non-zero exit, read the verdict with `verifique show`, fix the cause and submit again.
        Exit 2 means blocked or broken. Exit 3 means replaced or cancelled: do not submit it again.
    -   When submissions are refused because of needs-you, stop and tell the user. Do not move to a
        new branch to get round it.
    -   Call work done only on a pass for its final content.

### `doctor`

10. `verifique doctor` reads the project's `verifique.yaml`, says how many checks and stacks it
    declares and which `sh` runs its commands, and gives the version of each add-on the project
    uses, saying when one is behind verifique's own.
11. When any check uses the `claude` preset, it first confirms the `claude` command runs and is
    signed in, because an agent that cannot sign in fails only after a whole stack has been built.
12. It warns about settings in `.env` that point at a database on another machine and that
    `verifique.yaml` does not override. The warnings never block.
13. It pauses the queue while it runs: it waits for running checks to finish, starts no new ones, and
    lets them start again when it ends, however it ends. `verifique status` says the queue is paused.
14. For each stack, it builds once, then starts two copies side by side in the first two slots, each
    with fresh databases and the seed, waits until they are ready, runs the setup steps of the stack
    and of its checks, and confirms that each copy's databases were really used. Then it stops both
    and confirms nothing still answers on their ports, so no app left a process behind.
15. A database no copy used means the app still reaches the one in `.env`. `doctor` fails and says
    to point the setting the app reads at the slot's value, or to remove that database server if
    the app doesn't use it.
16. A stack with apps but no ready path is refused, because nothing could prove it started.
17. A stack with no apps is proved by seeding two copies. A database its seed leaves alone is
    accepted on the strength of the stack's settings, since nothing else runs before a check, and
    `doctor` says so.
18. A stack that needs a limited resource, such as one simulator, is proved with a single copy.
19. A stack in which two copies or two apps would share a port is refused.
20. It runs every step with the queue's `PATH`, as checks do, so it proves a stack with the tools its
    checks will get. Until a queue has run, it uses the caller's and says so.

## Never

-   verifique never writes the rules into the project. It only proposes them.
-   Neither verifique nor its guide ever installs a database server.

## Limits

-   `doctor` pauses every check on the machine while it runs, which on a slow build is minutes.
-   `doctor` only recognises settings that are database URLs. Split host settings, email and payment
    keys go unnoticed.
-   For a phone app, `doctor` proves only the backend. A real check has to prove the app reaches it,
    since an emulator cannot always use the host's address.
-   Setup is a one-off cost in the user's own agent time, a few dollars per project where it has
    been measured.

## Open questions

None recorded.
