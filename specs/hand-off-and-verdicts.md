---
status: active
last-reviewed: 2026-11-09
owner: Hlib Semeniuk
confidence: best-effort
related: [checks, queue, stacks, slots-and-isolation, setup]
glossary_terms: [Submission, Request, Brief, Content, Snapshot, Job, Check, Verdict, Passed, Failed, Blocked, Error, Replaced, Cancelled, Needs-you, Retry]
---

# The hand-off and verdicts

An agent that has finished a piece of work hands it to verifique with one command and carries on
with something else. Later it collects a verdict that belongs to the exact files it handed over,
with an exit code it can act on without reading prose.

## Why

Agents that finish together and each check their own work fight over the machine. Handing the work
over turns that fight into a queue: the agent moves on, and the machine runs the check when a slot
is free.

Waiting is a plain command that blocks until the verdict exists. It needs only a shell, so any agent
that can run a terminal command can use verifique, and an agent waiting in the background spends no
tokens.

The submission carries the user's request word for word, because the checker must judge the work
against what the user asked for, not against the author's idea of it.

A verdict belongs to the content, not to a branch or a commit. That way nobody has to commit before
checking, a pass can never be carried over to files that changed, and the same files committed later
need no second check.

## Behaviour

### Submitting

1. `verifique submit --check <name>` hands over the work in the current repository or worktree. When
   the project declares only one check, `--check` may be left out. Otherwise verifique asks which,
   listing the checks there are.
2. The request is given with `--request` or `--request-file`, and the brief with `--brief` or
   `--brief-file`. The request must be the user's own words, untrimmed.
3. A submission to an agent check without a request is refused. A command check accepts one without.
4. Uncommitted work is captured as a snapshot, without changing the branch, the index or any file in
   the user's checkout. Files git ignores are left out, as a commit would leave them out. `submit`
   says when it captured uncommitted work.
5. Everything about the check is fixed at the moment of submission. Editing `verifique.yaml`
   afterwards never changes a job that is already queued.
6. Anything after `--` is added to the end of a command check's `run`, each item quoted, such as the
   tests to run: `verifique submit --check e2e --request-file request.md --brief "…" -- tests/login.spec.ts`.
   An agent check refuses anything after `--`, because what it should look at belongs in the brief.
7. `submit` prints the job number and returns at once. When no queue is running it says so, and the
   job waits until one starts.
8. The checker receives the diff of the work, taken from where the work left its base branch.
9. `--headed` asks for a visible browser for this one run (see [checks](checks.md)).
10. A submission from a branch and check in needs-you is refused, with the reason, and nothing is
    queued (see [checks](checks.md)).

### Replacing and cancelling

11. A newer submission of the same check from the same branch replaces that branch's queued job of
    that check. A job that is already running is not replaced: it runs to its end.
12. `verifique cancel <job>` stops a queued or running job. A cancel is not a verdict and never
    counts against the branch.
13. Whoever waits on a replaced or cancelled job gets exit 3, and should not submit the same work
    again.

### Waiting

14. `verifique wait <job>` blocks until the job ends, then prints its result and summary.
15. When no queue is running, `wait` says so once and keeps waiting.

### Verdicts belong to content

16. A verdict is recorded against the content and the check. The same files, on any branch, in any
    worktree, committed or not, have the same verdict.
17. `verifique verdict [--check <name>]` reports the verdict recorded for the content in the current
    working tree, committed or not, without queueing anything.
18. A later result for the same content and check takes the place of the earlier one.
19. Every submission runs its check, even when that content already has a verdict.
20. The rules `verifique init` proposes tell agents to call work done only on a pass for its final
    content.

### Reading results

21. `verifique show <job>` gives one job in full: its check, branch, content, summary, anything added
    after `--`, and for an agent check the steps it took, what it saw and what it did not check,
    with the folder holding its files and evidence. For an error, it also shows the last lines of
    the log of whichever step ran last, because a one-line error rarely says why.
22. `verifique list` shows recent jobs, and `verifique status` shows the whole queue (see
    [queue](queue.md)).
23. Every command that reads state takes `--json`, for agents and for the panel.

### Exit codes

24. `wait` and `verdict` exit with:

    | Code | Meaning |
    |------|---------|
    | 0 | passed |
    | 1 | failed |
    | 2 | blocked or error, including a started stack that ended |
    | 3 | replaced or cancelled |

25. `verdict` exits 4 when the content has no verdict for that check.
26. `submit` exits 5 when it is refused because the branch and check are in needs-you.
27. Any other failure, such as an unknown job or a missing `verifique.yaml`, exits 64.

## Never

-   Capturing work never changes the user's branch, index or files.
-   A verdict never moves to different content.
-   Nothing forces an agent to use the queue. verifique proposes rules for the project's agent
    instructions and never writes them.

## Limits

-   The request is passed on by the agent that wrote the work, and verifique cannot tell whether it
    was trimmed or summarised.
-   An agent can dodge needs-you by moving to a new branch. The rules tell it not to.
-   Work submitted from no branch is never replaced by a newer submission and never goes to
    needs-you.

## Open questions

-   **Is `verifique verdict` part of the product?** It exists and works as described above, but the
    user guide, the rules and the web page never mention it. If it is, should the rules tell agents
    to ask it before submitting content that may already have passed? Settled by the owner.
-   **Does content that already passed count as verified without a new run?** The user guide and the
    decisions log say a later commit of the same content "already counts as verified". That holds for
    `verifique verdict`, but `submit` always runs the check again. Should `submit` hand back the
    existing pass instead? Settled by the owner.
-   **Which exit codes are a promise?** The user guide documents 0 to 3 for `wait`. Codes 4, 5 and 64
    are undocumented. A mistyped command exits 2, the same code as blocked or error, so an agent
    cannot tell a usage mistake from a blocked check by the code alone. Settled by deciding which
    codes are stable and documenting them.
-   **Is `base` part of `verifique.yaml`?** A top-level `base` key changes which branch the diff is
    taken against. No source documents it. Settled by the owner.
