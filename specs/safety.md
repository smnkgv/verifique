---
status: active
last-reviewed: 2026-11-09
owner: Hlib Semeniuk
confidence: best-effort
related: [databases, slots-and-isolation, checks, setup, queue, updates]
glossary_terms: [.env, Checker, Agent check, Setup step, Started stack, The service, Database server]
---

# Safety: what verifique touches and what it can't protect

A check runs the user's app unattended, the way the user would run it themselves. verifique
promises to touch only what it named, and says plainly what it cannot protect: whatever is live in
`.env`, credentials the app finds elsewhere, and what a checker sends its model provider.

## Why

Treat a check like starting the app yourself. A user lets checks run while they work on something
else, so they need to know exactly what a check can reach, and where the edges are, before they
connect a project. Hiding the risks would make verdicts cheaper and accidents likelier.

## Behaviour

### What verifique itself touches

1. On database servers, only the databases it named and, on Redis, the slot's own database and keys
   with the slot's prefix ([databases](databases.md) owns the rules).
2. Among processes, only those it started and those an earlier job left in the same slot copy
   ([slots and isolation](slots-and-isolation.md) owns the rules).
3. In the user's checkout, nothing: capturing uncommitted work never changes the branch, the index or
   any file.
4. In a project's code, only what the setup agent changes, as small edits on a branch shown to the
   user. verifique never writes the project's agent rules.
5. The service records only the variables tools need to be found, never credentials from the user's
   shell.
6. The settings a started stack keeps for `stack exec` include values from `.env`, so they are
   readable only by the user and removed when the stack stops.

### What is live

7. **`.env` is live.** Every copy of the app runs with the `.env` in the repository's main checkout,
   even for work submitted from a worktree. Only slot ports and the settings in `verifique.yaml` are
   replaced, so anything else in it is real: a check can send real email to real people, charge real
   cards or write to any service it points at. Installing the service says so before any project is
   connected.
8. `verifique doctor` names databases on other machines left in `.env`, but cannot see everything.
9. Cloud tools read credentials from the home folder as well as `.env`, so a check can reach a real
   account even with a clean `.env`. The setup guide tells the agent to point those at nothing unless
   a check needs them.
10. Whatever a stack's `setup` prints reaches the checker and its model provider, so it must only
    ever carry throwaway credentials.

### What leaves the machine

11. verifique has no account and no server of its own. Verdicts and their proof stay on the machine:
    there is no shared dashboard, and nothing appears on pull requests. Once a day the queue asks
    GitHub, where verifique's releases are published, for the latest version, sending nothing about
    the user or their projects, and `verifique updates off` stops it (see [updates](updates.md)).
12. An agent check sends the request, the diff and what it sees in the app to the agent's model
    provider, as the user's coding agent already does.
13. The app, its build and the checker's browser reach the network as they do when the user runs
    them.

### What a checker may do

14. A checker gets a browser and its own job folder, plus any tools the project adds under `tools`,
    on top of the user's own agent settings. Everything else follows the user's permission mode, so
    what a checker may do is the user's choice, as it is for any agent they run.
15. Nothing in the material a checker judges, the request, brief, diff, setup output or the app's
    own pages, can widen what it may do or tell it which verdict to give.

### Agents sharing one machine

16. The rules tell every agent never to start the app itself and never to stop processes by name or
    pattern, because other agents' work runs on the same machine under the same names.

## Never

-   verifique never bypasses the user's agent permissions.
-   It never sends anything to a server of its own.
-   It never updates itself without being asked.
-   It never drops a database it did not name.

## Limits

-   `doctor` recognises only database URLs. Split host settings, email and payment keys in `.env` go
    unnoticed.
-   A checker's powers depend on the user's agent settings, which differ from machine to machine.
-   A pass cannot vouch for security, performance or data integrity, which no screen shows.

## Open questions

None recorded.
