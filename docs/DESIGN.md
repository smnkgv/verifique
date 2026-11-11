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
   up. A stack needs no apps, so a suite that only needs its own databases doesn't build the app.
   Nothing runs on one before its check, so `doctor` takes its settings as proof for a database its
   seed leaves alone, and says so.
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
20. **Investigations take a slot.** An agent reproducing a bug starts a timed stack through the same
    queue, with the same isolation and no verdict. It stops by itself, so a forgotten stack or a
    crashed agent cannot jam the queue. It takes a slot as a check does: the slot count is how many
    copies run at once, whatever they are for. One slot used to be kept back for checks, but a count
    that doesn't mean what it says read as a broken panel.
    A person can ask for one of the project's `seeds` to fill it with real data; checks never do.
    `stack exec` runs a command in it with its settings, as its seed runs, for the project's own
    tools that need the apps up.
21. **Slots keep their worktrees,** and `clean` removes them. When free disk falls below a floor, 5 GB by default, the copies no check is using are deleted, and checks wait if that isn't enough.
22. **An installed background service.** Installing it records the `PATH` the user works with: the
    login shell's on macOS and Linux, the terminal's on Windows. A command shows what the service sees.

### Product

23. **A small core, written in Rust,** with add-ons as separate programs beside it (37).
24. **CLI, desktop notifications and JSON output.** Notifications fire only for `needs-you`, grouped
    per repository.
25. **A status-bar panel, as Docker has Docker Desktop.** The CLI is the core and works alone. The
    desktop install adds an icon with a simple dropdown of slots, the queue and what needs you, using
    Tauri so one codebase covers the macOS menu bar, the Windows notification area and Linux trays. The icon is amber with the number of checks left while they
    run, green after a pass, and red with a count when something needs you. The panel lists, in order, what needs you, what's running, what's queued and recent verdicts,
    with actions limited to viewing proof, handing a branch back and running a job next. It reads the
    core's `--json` output and acts only through the CLI. A CLI-only install stays available: the
    panel is an add-on, which `verifique panel install` fetches from the same release, checked
    against its checksums, puts where the OS keeps apps and opens at login, the same way on every
    OS. CI signs and notarizes the Mac app with an Apple Developer ID once its secrets exist, which
    also gives a `.dmg` for people who download it in a browser. Where
    an OS lets an app do less, the panel goes without rather than imitate macOS: Windows puts no text
    beside a tray icon, so the count is in a tooltip, and has no supported way to show a window on
    every desktop; Linux trays pass an app only its menu, so the panel opens from the menu; and
    neither blurs reliably behind a pinned panel, so it is less see-through there. On Wayland it runs
    through XWayland, because Wayland lets no window place itself or stay on top.
26. **Rehearsal:** three real repositories, web first and then native mobile, followed by an unrelated
    project, each from a stripped copy. A rehearsal passes on: setup by a fresh agent with no help, `doctor` passing,
    a real change reaching a verdict you have reviewed, at least 4 of 5 planted bugs caught with no good
    change failed, and startup and memory within about 1.5 times the hand-built stack.
27. **Hand-built stacks are replaced** once a repository's rehearsal passes, including how its suite
    gets stacks on a laptop. CI keeps its own path.
28. **Proof of the whole:** one real working day of about 15 sessions across all three repositories,
    with no stack collisions and verdicts worth trusting.
29. **Built in public.** Version 0.1 was released before that day, so the release says what has and hasn't been tested. Project configuration and sign-in scripts
    stay in each project's own repository, never here.

### Platforms

30. **macOS, Linux and Windows are equals.** One platform layer, `src/platform`, gives the rest of
    the code one interface, and each OS provides all of it in a module of its own. Nothing outside it
    names an OS. Docs, installs and releases treat the three alike.
31. **Commands are POSIX sh on every OS:** the system `sh` on macOS and Linux, and the `sh` that
    comes with Git for Windows, found through git because `bash.exe` in System32 is WSL's launcher.
    An interpreter built into verifique was tried on real configs first: `deno_task_shell` has no
    `eval`, `exec`, `if` or `for` and prints `${VAR:-x}` as text, `brush` has only placeholders for
    processes and pipes on Windows, and both would still need Git for Windows for `grep`, `sort` and
    the projects' own `.sh` scripts.
32. **A step and everything it starts end together.** On macOS and Linux it is a process group,
    asked to stop with SIGTERM, given 3 seconds, then killed. On Windows it is a Job Object, ended at
    once, because Windows has no signal a program without a window reliably answers. A job also ends
    when the queue dies, so nothing is left holding a slot's ports.
33. **The queue starts at logon, as the user, with no admin rights,** and restarts after a crash: a
    LaunchAgent, a systemd user unit, or a scheduled task. The task starts it through
    `verifique-service`, a launcher with no window, because every console program a task starts opens
    one. The launcher also brings the queue back whenever it ends, since a scheduled task only
    restarts a program that fails to start. All three log to `~/.verifique/server.log`.
34. **Each OS answers for its ports, processes and disk.** Who holds a port, a process's name and
    folder, and free disk come from libraries that ask the OS directly, with no `lsof` or `ps`.
    Windows has no tool that shows a process's folder, so asking it directly is the only way to tell
    a leftover of verifique's own from someone else's server.
35. **Notifications are the OS's own:** `osascript`, `notify-send`, or a toast under verifique's own
    name, which `service install` registers for the user.
36. **The settings `stack exec` reads stay with their owner:** mode 600 on macOS and Linux. On
    Windows the file keeps its folder's permissions, and the user's own folder is private.

### Add-ons

37. **Each technology is a plugin, installed on top of the core.** verifique should grow to many
    databases and more, and nobody should carry what their projects don't use. Postgres, Mongo and
    Redis are the programs `verifique-postgres`, `verifique-mongo` and `verifique-redis`, each with its
    own driver, so no client program is needed on any OS. They speak a versioned JSON protocol,
    described in [PLUGINS.md](PLUGINS.md), and a plugin refuses any database not named `verifique_*`.
38. **`verifique plugin add` installs them, the same way on every OS.** It fetches each plugin from
    the release verifique came from and checks it against that release's checksums, so versions
    match without a package per plugin in every package manager. `plugin list`, `plugin update` and
    `doctor` say when one is behind.

### Installing

39. **Each system installs verifique with its own package manager,** from repositories the project
    keeps itself, so upgrades come with the rest of the system: a Homebrew tap, a Scoop bucket, and
    for Linux, signed apt, dnf and pacman repositories on GitHub Pages. Each is made from the
    release's own builds, checked against its checksums, and the Linux ones are installed in clean
    Ubuntu, Debian, Fedora and Arch containers before they are published. An install script covers
    everything else. Distributions' own repositories need a maintainer to sponsor each package and
    months of review, and Snap and Flatpak run programs in a sandbox, while verifique has to start
    your apps and reach your databases and worktrees.

## Known limits

-   On Windows, commands need Git for Windows. Its `sh` rewrites arguments that look like Unix
    paths, such as `/api`, when it passes them to Windows programs, as Git Bash does everywhere.
-   On Windows, a step ends at once, with no grace period for it to tidy up.
-   Only macOS has run real projects so far.
-   The Postgres and Redis plugins don't speak TLS yet, so a server that requires it, such as
    `sslmode=require` or `rediss://`, is refused. Local servers rarely do.
-   Fixed slots and rules only leave capacity and adoption to the user.
-   `.env` is used as it is, so a setting the project file does not override can reach a real service.
    `doctor` only recognises database URLs: split host settings, email and payment keys go unnoticed.
-   Databases are separated per slot, but share one server, so one check's load can slow another.
-   Local suites and CI reach the same tests by different routes, so a failure can show in only one.
-   Proof shows that a check looked, not that it looked at the right thing. Each project's `ready`
    and `setup` steps have to test something real, and planted bugs measure how far to trust a pass.
-   A checker's powers depend on the user's agent settings, so verdicts can rest on different
    abilities from one machine to another.
-   Until CI has the Apple Developer ID's secrets, the Mac panel isn't signed or notarized, so it
    installs only through `verifique panel install`: macOS refuses a copy downloaded in a browser.
    The Windows panel isn't signed, so Windows warns about a copy downloaded in a browser.
-   The panel has run on macOS, and on Linux under X11 with a plain tray, but not yet in GNOME or
    KDE. On Windows it compiles, but hasn't run yet.
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
