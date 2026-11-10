---
status: active
last-reviewed: 2026-11-10
owner: Hlib Semeniuk
confidence: best-effort
related: [queue, slots-and-isolation, stacks, setup, add-ons, safety]
glossary_terms: [The service, Stack, Slot, Started stack, .env, Queue]
---

# Platforms

verifique runs on macOS, Linux and Windows as equals: the same commands, the same `verifique.yaml`
and the same promises on every system, with only the mechanisms underneath differing.

## Why

A project's `verifique.yaml` lives in its repository, so once one person has connected a project,
everyone who works on it has it, whatever system they use. It has to mean the same thing on each of
them, or a check that passes on one machine says nothing on another. Coding agents run on all three
systems, and so should the queue they hand work to.

## Behaviour

### Installing

1. verifique installs on macOS, Linux and Windows, on ARM and on Intel or AMD processors alike. On
   macOS and Linux it comes from Homebrew or an install script, on Windows from Scoop or an install
   script (see [setup](setup.md)).
2. Add-ons install the same way on every system (see [add-ons](add-ons.md)).

### One command language

3. Commands in `verifique.yaml` are POSIX sh on every system, so one file works everywhere. Paths in
   them are written with forward slashes, which all three understand.
4. On macOS and Linux they run in the system's own sh.
5. On Windows they run in the sh that comes with Git for Windows, which makes Git for Windows a
   prerequisite there. When it is missing, verifique says so and how to install it.
   `verifique doctor` names the sh it uses.

### Stopping what a step started

6. Stopping a step stops everything it started, on every system, including processes its processes
   started. When the queue itself dies, what its jobs started ends with it, so nothing is left
   holding a slot's ports.
7. On macOS and Linux, what a step started is first asked to stop and gets 3 seconds before it is
   ended.
8. On Windows, it is ended at once, with no grace period.

### The queue in the background

9. On every system, the queue starts at logon as a service for the current user, needs no
   administrator rights, runs without opening a window, and comes back after a crash. All three
   keep the queue's log in the same place in the user's verifique folder.

### Notifications

10. Notifications come from the system's own notification mechanism. On Windows they appear under
    verifique's own name, which installing the service registers for the user.

### Files only the user can read

11. The settings a started stack keeps for `stack exec` hold values from `.env`, so only the user may
    read them. On macOS and Linux the file's own permissions say so. On Windows, verifique relies on
    the permissions of the user's own folder, which other users cannot read, and sets nothing on the
    file itself.

### Promises that hold everywhere

12. Everything the other specs promise holds on every system: slots and their ports, fresh
    databases and the prefix rule, naming whatever holds a slot's port, the disk floor, verdicts tied
    to content, exit codes and `--json` output.

## Never

-   The service never needs administrator rights.
-   A project never needs a different `verifique.yaml` for a different system.

## Limits

-   Real projects have run only on macOS so far. Linux and Windows pass verifique's own tests and its
    example project.
-   On Windows, a project's commands depend on Git for Windows being installed.
-   On Windows, Git's sh rewrites arguments that look like Unix paths, such as `/api`, when it passes
    them to Windows programs, as Git Bash does everywhere.
-   On Windows, apps get no grace period to shut down cleanly when a step is stopped.
-   The panel does less on Windows and Linux, which let an app do less: on Windows the count is in
    a tooltip and a pinned panel stays on one desktop, and Linux trays open the panel from a menu
    (see [queue](queue.md)).

## Open questions

-   **When does a system count as proven?** The user guide says verifique runs on all three, and
    that real projects have run only on macOS. Is a rehearsal on real projects, like the Mac's,
    owed on Linux and Windows before that limit is lifted? Settled by the owner.
