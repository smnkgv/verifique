---
status: active
last-reviewed: 2026-11-10
owner: Hlib Semeniuk
confidence: best-effort
related: [slots-and-isolation, stacks, setup, safety, add-ons]
glossary_terms: [Database server, Slot, Slot number, Seed, Stack, .env, Doctor]
---

# Databases

Each slot gets its own databases on the database servers the user already runs, created fresh for
every check and dropped afterwards. verifique only ever creates and drops databases it named itself,
with the `verifique_` prefix. Redis, which has numbered databases instead of named ones, gives each
slot the database whose number matches it.

## Why

Fresh databases for every check mean the data always matches the code, and a verdict never depends
on what an earlier check left behind. One set per slot means copies running side by side never trip
over each other's data.

The database servers are a prerequisite. The user already runs them, however they installed them,
and installing or managing servers would make verifique slower, larger and riskier. verifique only
borrows space on them.

The prefix is the safety line. Because verifique never creates or drops a database without it, a
slot's database can never be a real one, whatever a project's settings say.

## Behaviour

### Supported servers

1. verifique supports Postgres, MongoDB and Redis, declared under `services` in `verifique.yaml` as
   `postgres`, `mongo` and `redis`. Each takes an optional `url`, and without one verifique uses the
   server's usual local address. Each kind comes from an [add-on](add-ons.md) of the same name.
2. A stack that uses a server's values must declare that server under `services`.
3. The server must already be running. verifique never installs or starts one. A server it cannot
   reach ends the job in an error saying that a running database server is a prerequisite, with
   any password hidden.
4. Each add-on talks to its server itself, so no database client program, such as `psql`, is
   needed. A kind whose add-on is not installed ends the job in an error naming the add-on and the
   command that adds it.

### One set of databases per slot

5. For each job with a stack, every declared server gets a database for that repository and slot,
   named `verifique_<repository>_s<slot number>`. The repository's name is lowercased, anything but
   letters and digits becomes `_`, and a long name is shortened so the whole fits the server's
   limit while keeping the prefix and the slot.
6. Before the seed runs, the slot's database is dropped and, on Postgres, created empty. MongoDB and
   Redis create what they need on first write. When the job ends, the database is dropped again. If
   that fails, the next check in the slot puts it right.
7. A check without a stack gets no databases.

### The prefix rule

8. verifique only ever creates and drops databases whose names start with `verifique_`.

### Redis

9. Each slot uses the numbered Redis database that matches its slot number, and clears it for every
   check. Nothing else should be kept in those databases.
10. Each slot also gets a key prefix, `verifique_<repository>_s<slot number>:`, for apps that prefix
    their keys instead of choosing a database. Keys with that prefix in database 0 are deleted for
    every check, and nothing else in database 0 is touched.
11. Slot 0 is refused for Redis, because database 0 is the one everything else on the machine shares.
12. A slot whose number the Redis server has no database for is refused, with an error saying to
    raise the server's number of databases or start the queue with a lower first slot. verifique
    makes sure the database exists before clearing it, so a missing database can never lead to
    clearing another.

### Values for the project's commands

13. For each server, commands get `{<kind>.url}` (the slot's own database), `.server`, `.database`,
    `.host`, `.port`, `.user` and `.password`. Redis adds `.db`, the database number, and `.prefix`,
    the key prefix: an app that takes a database number uses `.url`, and one that prefixes its keys
    uses `.prefix`.

### Databases left in `.env`

14. `verifique doctor` warns about settings in `.env` that point at a database on another machine
    and that `verifique.yaml` does not override (see [setup](setup.md)).
15. `doctor` fails a stack whose copies never used one of their databases, because the app is still
    reaching the database in `.env`.

## Never

-   verifique never installs, starts or reconfigures a database server.
-   It never creates or drops a database without the `verifique_` prefix.
-   It never clears a Redis database other than the slot's own, and never clears database 0.

## Limits

-   Each slot's databases are separate, but they share one server, so one check's load can slow
    another.
-   Only three kinds of database are supported so far. More come as [add-ons](add-ons.md).
-   The Postgres and Redis add-ons do not speak TLS yet, so a server that requires it is refused.
    The MongoDB add-on does. Local servers rarely require it.
-   Redis servers usually have 16 numbered databases, so with the default first slot of 4, a machine
    can run at most 12 slots against Redis without raising that number.
-   `doctor` recognises only database URLs, not split host settings.

## Open questions

-   **Should users know about the keys in Redis database 0?** The user guide describes the numbered
    database each slot owns, but not that prefixed keys in database 0 are deleted for every check.
    The behaviour keeps to the prefix rule, yet it touches the database the slot-0 refusal exists to
    protect. Settled by the owner confirming it and the guide saying so.
