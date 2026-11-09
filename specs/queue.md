---
status: active
last-reviewed: 2026-11-10
owner: Hlib Semeniuk
confidence: best-effort
related: [hand-off-and-verdicts, checks, slots-and-isolation, stacks, platforms]
glossary_terms: [Queue, The service, Slot, Job, Retry, Resource, Disk floor, Needs-you, Panel, Started stack]
---

# The queue and its service

One queue per machine serves every repository on it. It runs only as many checks at once as the
user allows, takes repositories in turn, and runs in the background from logon. A status-bar panel
shows it at a glance.

## Why

Projects share the machine's memory and ports, so the queue belongs to the machine, not to a
project. Fifteen agents that finish together would otherwise mean fifteen builds, fifteen copies of
the app and fifteen browsers at once: memory runs out, and checks fail because the machine is
overloaded, not because the code is wrong.

A fixed number of slots, two by default, is simple and predictable, and leaves capacity to the user,
who can raise it when the machine has room. The price is waiting a turn when many agents finish
together.

Repositories take turns so one busy repository cannot starve the others. Within a repository, a
retry goes first because that agent's context is freshest.

A check that runs out of disk fails in ways that look like a fault in the change, so verifique frees
disk first and makes checks wait rather than letting them fail.

## Behaviour

### One queue for the machine

1. One queue serves every project on the machine, and knows nothing about their frameworks or tests.
2. `verifique service install` installs the queue as a background service for the current user,
   starts it, and starts it again at every logon. It takes `--slots` (default 2), `--resource
   <name>=<count>` (repeatable), `--min-free-gb` (default 5) and `--first-slot` (default 4), and
   keeps them for every later start.
3. Installing again replaces the service. `verifique service restart` restarts it, such as after a
   new build. `verifique service uninstall` stops and removes it. `verifique serve` runs the queue in
   a terminal instead, with the same options.
4. Installing records the `PATH` the user works with (on macOS and Linux their login shell's, on
   Windows their terminal's) and the few other variables tools need to be found, such as
   the home folder, the language and SDK locations, and never credentials from the shell. Every step
   runs with them. `verifique service env` shows what the service sees and which common tools it can
   find.
5. A second queue cannot start while one is running.
6. When no queue is running, `submit`, `wait`, `stack start`, `status` and a `cancel` of a running
   job say so, and say how to start one.

### Slots

7. The queue runs at most as many jobs at once as it has slots.
8. `verifique slots <n>` changes the number while the queue runs, to at least one. Lowering it never
   stops a running check. Restarting the service goes back to the installed number.

### Who goes next

9. When a slot comes free, `verifique next <job>` jobs go first. Otherwise the repository that was
   served longest ago goes next, and within it a retry goes ahead of new work, then the oldest job.
10. A job waiting for a resource that is in use never holds up a job that fits.
11. A check that needs several slots gets as many as the machine has. While it waits for enough to
    come free, nothing else starts, so smaller jobs cannot keep it waiting for ever.
12. A job that needs a resource the machine does not have at all ends at once in an error naming it.
13. Started stacks never take the last slot (see [stacks](stacks.md)).

### Disk

14. When free disk falls below the floor, 5 GB unless `--min-free-gb` says otherwise, no check
    starts. verifique deletes the slot copies no running job uses, and checks wait until there is
    room. `verifique status` says so while jobs wait.

### Restarts and upgrades

15. When the queue stops, it stops its running jobs. At the next start, an interrupted check runs
    again from the start, a check that was being cancelled ends cancelled, and a started stack ends.
16. A queued job that a different version of verifique wrote, and this one cannot read, ends in an
    error asking for it to be submitted again, after anything it left running is stopped.

### Notifications

17. verifique raises a desktop notification only when a branch lands in needs-you.
18. Each repository raises at most one notification a minute. Any more in that minute are summed up
    in one, such as "3 more need you", so a broken setup that sends every branch to needs-you at
    once raises two notifications, not dozens.
19. A notification that fails never affects a verdict. `verifique status` stays the source of truth.

### Seeing the queue

20. `verifique status` shows whether the queue is running, how many slots are busy, what is running
    and where, what is queued, what needs you and why, and the latest verdicts. It also says when
    checks wait for disk or for `doctor`.
21. `verifique list` shows recent jobs and `verifique show <job>` shows one in full (see
    [the hand-off](hand-off-and-verdicts.md)). All three take `--json`.
22. Slots keep their copies of the code between checks, so builds stay fast. `verifique clean`
    removes them (see [slots and isolation](slots-and-isolation.md)).

### The panel

23. On macOS, a small separate app puts a tick in the menu bar. Clicking it lists what needs you,
    what is running, what is queued and the recent verdicts with their proof.
24. From the panel, a person can view a job's proof, hand a branch back, run a queued job next and
    change how many checks run at once.
25. The tick is amber with the number of checks left while they run, green for five minutes after the
    last one passes, and red with a count when something needs you.
26. Pinning the panel keeps it on screen in the top right corner, semi-transparent, on every desktop.
27. The panel reads the CLI's `--json` output and acts only through the CLI, so it can do nothing the
    CLI cannot. The CLI works alone, and installing without the panel stays possible.

## Never

-   Lowering the slot count never stops a running check.
-   The panel never acts other than through the CLI.
-   A notification never decides anything.

## Limits

-   Fixed slots and rules only leave capacity and adoption to the user.
-   Waiting a turn costs time: with agent checks of about five minutes and two slots, the last of
    fifteen agents that finish together waits about forty minutes.
-   The panel is built and signed for the user's own machine only. Sharing it needs an Apple
    developer identity.
-   There is no panel for Linux or Windows yet. The CLI does everything the panel does.

## Open questions

-   **One queue per machine, or one per home?** The user guide says one server runs per machine.
    A queue given a home folder of its own (the `VERIFIQUE_HOME` variable) runs beside the default
    one, with its own service, and verifique's own self-check relies on that. Is a separate queue a
    supported feature, or only for testing? Settled by the owner.
-   **What can the panel do?** The decisions log limits the panel's actions to viewing proof, handing
    a branch back and running a job next. The user guide and the panel itself also change the slot
    count, which the decisions log allows elsewhere. Settled by aligning the log.
