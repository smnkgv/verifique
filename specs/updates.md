---
status: active
last-reviewed: 2026-11-11
owner: Hlib Semeniuk
confidence: best-effort
related: [queue, add-ons, platforms, safety, setup]
glossary_terms: [Update notice, The service, Queue, Panel, Add-on, Started stack]
---

# Updates: knowing a newer verifique is out, and updating in one step

Once a day the queue asks GitHub, where verifique's releases are published, for the latest version.
When a newer one is out, the panel says so with an Update button, and `verifique status` says to run
`verifique update`. Either one brings verifique, the queue, its add-ons and the panel to the new
version, the way verifique was installed, once the person has said so. verifique never updates
without being asked, and `verifique updates off` stops the asking.

## Why

People install verifique once and then forget about it. Package managers upgrade it only when
someone asks them to, and testers rarely do, so they run old versions with fixed bugs without
knowing. Telling them where they already look, the panel and `status`, costs one small request a
day.

Telling is not enough if updating is a chore. Each way of installing has its own upgrade command,
and an upgrade is not finished until the queue restarts and the add-ons and panel follow, or they
run different versions side by side, which is how a fixed bug stays unfixed. So one step does all
of it, from the panel for the people who live there, and from one command for those in a terminal.

The person still chooses when, because restarting the queue ends started stacks, and only they know
whether one is in use.

The request goes to GitHub rather than to a server of verifique's own, which keeps the promise that
verifique has none (see [safety](safety.md)). It is still a call to the internet, so it says so and
can be turned off.

## Behaviour

### Asking

1. While the queue runs, it asks GitHub for verifique's latest published release at most once a
   day, with no command waiting on the answer. A release still being published counts only once
   it is out.
2. When the asking fails, such as with no network, the queue tries again an hour later.
3. The queue sends nothing but the request itself: nothing about the user, their projects or
   their checks.
4. Nothing asks while no queue runs.

### Telling

5. When the latest release is newer than the verifique a person runs, `verifique status`,
   `verifique doctor`, `verifique plugin list` and `verifique updates` end with one line saying so
   and to run `verifique update`, every time they run, until it is updated.
6. The line goes to the error output and never appears with `--json`, nor on `submit`, `wait`,
   `show`, `list`, `verdict` or `stack`, which agents read.
7. Once verifique is updated but the queue still runs an older version, or an add-on verifique
   publishes is behind it, the line says so and to run `verifique update` to finish.
8. A build from source says to pull and build again instead, and an install nothing on the machine
   explains points at the install guide.

### In the panel

9. While any of those is waiting, the panel shows one line above its footer, such as
   "verifique 0.3.4 is out", with an Update button, and its tick gets a blue dot. On Windows the
   icon's tooltip says it too.
10. Clicking Update while checks or started stacks are running first says what restarting the queue
    will do, such as "1 started stack stops, and 2 running checks start again from the beginning",
    with Update now and Not now. With nothing running it updates at once.
11. While it updates the line says so, and a failure stays in the panel with its reason. When the
    panel itself was updated, it closes and opens again at the new version.
12. A build from source, or an install nothing explains, gets a link to what the release changed
    instead of a button.
13. The panel learns all this from the CLI and updates through it, so it never asks GitHub itself
    and can do nothing `verifique update` can't.

### Updating

14. `verifique update` asks GitHub for the latest release there and then, and brings everything to
    it, in order: verifique itself, the queue, the add-ons verifique publishes, then the panel. It
    does only the steps still needed, and says so when there are none.
15. verifique itself is updated the way it was installed:
    -   with Homebrew, by Homebrew's own upgrade;
    -   from the apt, dnf or pacman repository, by that package manager, which asks for the
        administrator's password: in the terminal when run there, and in the system's own password
        window when run from the panel;
    -   with an install script, and on Windows however it was installed, by fetching that release's
        files for this machine, checking them against the release's checksums and putting them in
        place of its own, as the install scripts do.
16. When the package manager doesn't have the new version yet, nothing more happens, and
    verifique says to try again later.
17. Restarting the queue ends started stacks, and running checks start again from the beginning
    (see [queue](queue.md)). So in a terminal `verifique update` says what will stop and asks before
    it changes anything, and anywhere else it refuses unless given `--yes`, which the panel passes
    once the person has agreed there. An agent, which has no terminal, cannot update verifique.
18. A queue run in a terminal with `verifique serve` is left running, and verifique says to restart
    it there.

### Turning it off

19. `verifique updates off` stops the queue asking at once, with no restart, and silences the
    notice of a newer release in the CLI and the panel. `verifique updates on` turns it back on, and
    the queue asks again straight away.
20. With it off, the reminders in 7 still appear, because they come from the machine itself, not
    the internet, and `verifique update` still works when someone runs it.
21. `verifique updates` says whether it is on, when the queue last asked and what it found. With
    `--json` it says the same, and what the panel shows, for the panel.

## Never

-   verifique never updates itself without a person asking for it.
-   It never restarts the queue without saying first what that stops.
-   An update notice never appears in output an agent reads.

## Limits

-   With no queue running, nothing asks, so a machine that never starts one is never told.
-   On Windows, verifique puts the new files in place itself, because Scoop won't update a program
    while an agent waits on a check. `scoop list` shows the old version until Scoop next updates it.
-   pacman updates verifique on its own rather than the whole system, which Arch advises against.
    verifique needs nothing but git, so nothing else depends on its version.
-   Linux needs a desktop that shows the system's password window for the panel to update a
    package; otherwise `verifique update` in a terminal does it.
-   On Linux with Homebrew, the service is recorded at the program's own versioned folder, which
    Homebrew removes after an upgrade, so restarting the queue fails there until
    `verifique service install` runs again. macOS records the link Homebrew keeps.
-   GitHub limits how often one address may ask without signing in. One request a day is far below
    that, but many machines behind one address share the allowance.

## Open questions

-   **Should the queue restart when it is next idle?** Updating now asks first and then restarts at
    once. Waiting until no check runs would spare running checks, at the cost of an update that
    lands later and a queue paused meanwhile. The owner chose asking first; worth revisiting if
    testers often update with checks running.
