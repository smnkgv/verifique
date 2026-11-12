# Design

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
    slot's ports must be free as well: a process an earlier job left in the slot is stopped, and
    anything else on one of them ends the job with an error naming it, so a check never runs against
    someone else's server.
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
    while the server runs, from the CLI or the panel. Lowering it never stops a running check, and a
    restart goes back to the installed number.
18. **A check can take several slots,** for a suite with several workers. It gets as many as the
    machine has, sharing one build, and the scheduler holds slots back so it cannot starve.
19. **Apps use the project's own `.env`,** as a user would. Slot values override it. Installing warns
    about the risk, and `doctor` names remote databases left in place without blocking.
20. **Investigations take a slot.** An agent reproducing a bug starts a timed stack through the same
    queue, with the same isolation and no verdict. It stops by itself, so a forgotten stack or a
    crashed agent cannot jam the queue. It counts against the slots as a check does: one slot used to
    be kept back for checks, but a count that didn't mean what it said looked like a broken panel. A
    person can load real data into it with one of the project's `seeds`, which checks never use, and
    `stack exec` runs the project's own tools in it. `stack rebuild` puts newer work into it, keeping
    its slot and data, and rebuilds the production build rather than running a dev server, so what
    an agent sees by hand is what a check sees.
21. **Slots keep their worktrees,** and `clean` removes them. Below a disk floor, 5 GB by default,
    copies no check is using are deleted, and checks wait if that isn't enough.
22. **An installed background service.** Installing it records the `PATH` the user works with: the
    login shell's on macOS and Linux, the terminal's on Windows.

### Product

23. **A small core, written in Rust,** with add-ons as separate programs beside it (37).
24. **CLI, desktop notifications and JSON output.** Notifications fire only for `needs-you`, grouped
    per repository.
25. **A status-bar panel, as Docker has Docker Desktop.** The CLI is the core and works alone. The
    panel is a Tauri app, so one codebase covers the macOS menu bar, the Windows notification area
    and Linux trays. It reads the CLI's `--json` output and acts only through the CLI, and it is an
    add-on that `verifique panel install` fetches from the same release. [The queue
    spec](../specs/queue.md) says what it shows and does. CI signs and notarizes the Mac app, which
    also gives a `.dmg`. Where an OS lets an app do less, the panel goes without rather than imitate
    macOS: Windows shows no text beside a tray icon and can't show a window on every desktop, Linux
    trays open the panel from a menu, and neither blurs reliably behind a pinned panel. On Wayland it
    runs through XWayland, because Wayland lets no window place itself or stay on top.
26. **Rehearsal:** three real repositories, web first and then native mobile, followed by an
    unrelated project, each from a stripped copy. A rehearsal passes on: setup by a fresh agent with
    no help, `doctor` passing, a real change reaching a verdict you have reviewed, at least 4 of 5
    planted bugs caught with no good change failed, and startup and memory within about 1.5 times the
    hand-built stack.
27. **Hand-built stacks are replaced** once a repository's rehearsal passes, including how its suite
    gets stacks on a laptop. CI keeps its own path.
28. **Proof of the whole:** one real working day of about 15 sessions across all three repositories,
    with no stack collisions and verdicts worth trusting.
29. **Built in public.** Version 0.1 was released before that day, so the release says what has and
    hasn't been tested. Project configuration and sign-in scripts stay in each project's own
    repository, never here.

### Platforms

30. **macOS, Linux and Windows are equals.** One platform layer, `src/platform`, gives the rest of
    the code one interface, and each OS provides all of it in a module of its own. Nothing outside it
    names an OS. Docs, installs and releases treat the three alike.
31. **Commands are POSIX sh on every OS:** the system `sh` on macOS and Linux, and the `sh` that
    comes with Git for Windows, found through git because `bash.exe` in System32 is WSL's launcher.
    Two shells that could be built into verifique, `deno_task_shell` and `brush`, were tried on real
    configs first. Neither could run them, and both would still have needed Git for Windows for
    `grep`, `sort` and the projects' own `.sh` scripts.
32. **A step and everything it starts end together.** On macOS and Linux it is a process group,
    asked to stop with SIGTERM, given 3 seconds, then killed. On Windows it is a Job Object, ended at
    once, because Windows has no signal a program without a window reliably answers. A job also ends
    when the queue dies, so nothing is left holding a slot's ports.
33. **The queue starts at logon, as the user, with no admin rights,** and restarts after a crash: a
    LaunchAgent, a systemd user unit, or a scheduled task. On Windows the task starts
    `verifique-service`, a launcher with no window that also brings the queue back whenever it ends,
    which a scheduled task alone does not. All three log to `~/.verifique/server.log`.
34. **Each OS answers for its ports, processes and disk** through libraries that ask it directly,
    with no `lsof` or `ps`. On Windows that is the only way to see a process's folder, which tells a
    leftover of verifique's own from someone else's server.
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
    keeps itself: a Homebrew tap, a Scoop bucket, and signed apt, dnf and pacman repositories on
    GitHub Pages. Each is made from the release's own builds, checked against its checksums, and the
    Linux ones are installed in clean containers before they are published. An install script covers
    everything else. Distributions' own repositories need a sponsor for each package and months of
    review, and Snap and Flatpak run programs in a sandbox, while verifique has to start your apps
    and reach your databases and worktrees.

### Stacks and doctor

40. **A stack names the host its apps answer at,** `127.0.0.1` unless it says otherwise. Some apps
    accept cookies or origins at one name only, such as `localhost`, and one setting then replaces
    a rule every agent on that project would otherwise have to remember.
41. **`doctor` runs with the queue's `PATH`,** which the queue records when it starts, because it
    proves what checks will do, and a check gets the queue's tools, not those of whoever runs
    `doctor`.

### Updates

42. **verifique says when a new release is out, and updates in one step when asked.** People
    install it once and forget, and package managers upgrade only when asked. The queue asks GitHub
    for the latest release once a day and records the answer, and commands and the panel only read
    that record, so none waits on the network. It is the only request verifique makes to the
    internet unasked, it goes to GitHub rather than a server of the project's, and
    `verifique updates off` stops it. The notice goes to stderr, and the panel reads it from
    `verifique updates --json`, so nothing an agent parses changes. An upgrade isn't finished until
    the queue restarts and the add-ons and panel follow, so `verifique update`, which the panel's
    Update button runs, does all of it: the upgrade the way verifique was installed, told from where
    the program is, then the rest, each run by the new program. Install scripts and every Windows
    install get the release's archive, checked against its checksums. That includes Scoop, which
    won't update while an agent's `wait` runs. Nothing more happens until the new program reports
    the new version, since a tap or repository can lag behind the release. Restarting the queue ends
    started stacks, so `update` says what will stop and asks first, and without a terminal it needs
    `--yes`, which keeps agents from updating.

## Known limits

Each spec in [specs/](../specs/_index.md) ends with the limits of its own area.
