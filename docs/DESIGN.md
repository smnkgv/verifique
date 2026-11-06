# Design

Licensed under Apache-2.0. The project is owned personally by its author.

## The problem

One laptop runs swarms of coding agents across several repositories. Verifying locally gives the
fastest feedback, but when agents verify at once their stacks fight for ports and memory. This tool
lets agents hand verification over and carry on, while the laptop runs each check when a slot is
free.

Isolation and "heavy commands take turns" are already served by tools such as heavy, agentgc, isola
and Coasts. This project is the part they leave open: the hand-off, an independent check of exact
content, and a verdict with proof.

## Decisions

### Scope

1. **Any project, from nothing.** Rehearsed on stripped copies of three real repositories, then on one
   unrelated open-source project.
2. **The tool does the isolation.** A project declares its apps and the database servers they use in
   one short file. The tool gives each slot its own ports and databases, starts the apps and cleans
   up.
3. **Database servers are a prerequisite.** The tool uses what is already there, however it was
   installed, and never installs one. It only ever creates and drops databases named `verifique_*`.
4. **Setup by the user's own agent.** `init` prints a guide for writing the project file. The agent
   may make small edits so the app reads config from the environment, on a branch for review, and
   stops on anything bigger. `doctor` proves the result: two stacks side by side, each using its own
   database.

### The hand-off

5. **Verdicts return through a background `wait`.** It needs only a shell, so it works with any agent.
6. **A submission carries the original request word for word and a brief.** An agent check refuses
   a submission without the request. The checker also receives the diff.
7. **Uncommitted work is snapshotted.** Verdicts are tied to the content (the git tree), so a later
   commit of identical content already counts as verified.
8. **Rules only.** `init` proposes the wording for the project's agent instructions, and never writes
   it. Nothing forces an agent to use the queue.

### Checking

9. **A built-in checker template** drives any agent command. Plain commands work too.
10. **Each check starts clean.** Its databases are created fresh, the project's `seed` runs on them,
    and they are dropped afterwards. Seeding runs every time so data always matches the code. A
    slot's ports must be free as well: a process left in the job's worktree by an earlier job is
    stopped, and anything else on one of them ends the job with an error naming it, because
    otherwise the check would run against that process instead of the code it came to verify.
11. **A project `setup` step** runs once the stack is ready. Its output reaches the checker, so it
    must only ever carry throwaway credentials.
12. **A checker runs with the user's own agent settings.** It gets a browser and can write its verdict
    and evidence, and a project can add tools. Everything else follows the user's permission mode,
    so what a checker may do is their choice, as it is for any agent they run.
13. **Headless by default, headed when it helps.** A check sets its browser mode, and a submission
    can override it, so a person can watch a flow.
14. **A pass from an agent needs proof:** steps, what it saw and at least one evidence file.
    Otherwise the job is an `error`. For a plain command, exit 0 is the proof.
15. **Three failures in a row send a branch to `needs-you`.** `blocked` goes there at once. An
    `error` does not count against the agent. A pass resets the count.

### Running

16. **Fair across repositories, retries first.** Repositories take turns for free slots. Within one,
    a resubmission after a failure goes first, then oldest first. `next` overrides this for one job.
17. **Fixed slots.** The number is set per machine and defaults to 2. `verifique slots <n>` changes it
    while the server runs, and the status-bar panel uses the same command. Lowering it never stops a
    running check. A restart of the service goes back to the installed number.
18. **A check can take several slots,** for a suite with several workers. It gets as many as the
    machine has, sharing one build, and the scheduler holds slots back so it cannot starve.
19. **Apps use the project's own `.env`,** as a user would. Slot values override it. Installing warns
    about the risk, and `doctor` names remote databases left in place without blocking.
20. **Investigations hold a slot.** An agent reproducing a bug takes a timed hold through the same
    queue, with the same isolation and no verdict. It expires by itself, so a forgotten hold or a
    crashed agent cannot jam the queue. One slot always stays free of holds, so checks keep moving.
21. **Slots keep their worktrees.** There is a manual `clean`, with no caps and no automatic deletion.
22. **An installed background service.** Installing it records the shell's `PATH`, and a command
    shows what the service sees.

### Product

23. **A single binary, written in Rust.**
24. **CLI, desktop notifications and JSON output.** Notifications fire only for `needs-you`, grouped
    per repository.
25. **A status-bar panel, as Docker has Docker Desktop.** The CLI is the core and works alone. The
    desktop install adds an icon with a simple dropdown of slots, the queue and what needs you, using
    Tauri so one codebase covers the macOS menu bar first, then the Windows tray and Linux. The icon shows a count only when something needs
    you. The panel lists, in order, what needs you, what's running, what's queued and recent verdicts,
    with actions limited to viewing proof, handing a branch back and running a job next. It reads the
    core's `--json` output and acts only through the CLI. A CLI-only install stays available. The
    macOS panel is built. Windows also needs the core ported.
26. **Rehearsal:** three real repositories, web first and then native mobile, followed by an unrelated
    project, each from a stripped copy. A rehearsal passes on: setup by a fresh agent with no help, `doctor` passing,
    a real change reaching a verdict you have reviewed, at least 4 of 5 planted bugs caught with no good
    change failed, and startup and memory within about 1.5 times the hand-built stack.
27. **Hand-built stacks are replaced** once a repository's rehearsal passes, including how its suite
    gets stacks on a laptop. CI keeps its own path.
28. **Proof of the whole:** one real working day of about 15 sessions across all three repositories,
    with no stack collisions and verdicts worth trusting.
29. **Built in public, released once that day passes.** Project configuration and sign-in scripts
    stay in each project's own repository, never here.

## Known limits

-   macOS and Linux only: process groups and `sh`.
-   Fixed slots, rules only and no disk cap leave capacity, adoption and disk to the user.
-   `.env` is used as it is, so a setting the project file does not override can reach a real service.
    `doctor` only recognises database URLs: split host settings, email and payment keys go unnoticed.
-   Databases are separated per slot, but share one server, so one check's load can slow another.
-   Local suites and CI reach the same tests by different routes, so a failure can show in only one.
-   Proof shows that a check looked, not that it looked at the right thing. Each project's `ready`
    and `setup` steps have to test something real, and planted bugs measure how far to trust a pass.
-   A checker's powers depend on the user's agent settings, so verdicts can rest on different
    abilities from one machine to another.
-   The panel is signed for one machine: sharing it needs an Apple Developer ID.
-   `doctor` pauses every check on the machine while it runs, which on a slow build is minutes.
-   A server a checker starts by hand can outlive its job, using memory until the next job in that
    slot stops it.
-   An agent can dodge the failure limit by moving to a new branch.
-   The request is passed on by the writing agent, and the tool cannot tell if it was trimmed.
-   Only the Claude Code checker preset has been run. Whether Codex wakes on a finished background
    command, and how finely it can limit tools, is unconfirmed.
-   The Claude checker preset needs the `claude` command signed in on its own: a desktop session's
    sign-in does not reach it. `doctor` checks this first.
-   A project that already has its own device broker could keep it or replace it. Decide per project.
