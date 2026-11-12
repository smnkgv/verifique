---
status: active
last-reviewed: 2026-11-10
owner: Hlib Semeniuk
confidence: best-effort
related: [databases, platforms, setup, updates]
glossary_terms: [Add-on, Database server, Slot]
---

# Add-ons

Support for each kind of database is an add-on: a separate program installed on top of verifique's
core with `verifique plugin add <name>`, the same way on every system, from the same release as the
core. The status-bar panel is an add-on too, installed with `verifique panel install` (see
[queue](queue.md)). People install only what they use.

## Why

verifique should grow to support many databases, and more than databases, without everyone carrying
all of it. A small core with add-ons keeps each install to what that machine's projects need, the way
other tools ship a core package with adapters on top. Fetching add-ons from verifique's own release
keeps their versions matched to the core without a package for each one in every package manager.

## Behaviour

### Installing and keeping add-ons

1. Postgres, MongoDB and Redis support are the add-ons `postgres`, `mongo` and `redis`.
   `verifique plugin add postgres mongo redis` installs them, the same way on macOS, Linux and
   Windows.
2. `plugin add` fetches each add-on from the release the core itself came from and checks it against
   that release's checksums. One that does not match is not installed. A name the release does not
   publish is refused, with the names it does.
3. Add-ons go in one folder per user, whichever queue they serve.
4. `verifique plugin list` shows each add-on, where it is and its version, or that it is not
   installed and the command that adds it.
5. After the core is upgraded, `verifique plugin update` brings every installed add-on verifique
   publishes to the core's version. `plugin list` and `verifique doctor` say when one is behind, and
   so do `verifique status` and the panel once the queue runs the new core. `verifique update`
   brings them along with the core (see [updates](updates.md)).
6. `verifique plugin remove <name>` removes an add-on `plugin add` installed. For one installed some
   other way, it says where it is and leaves it.

### Using add-ons

7. A project uses an add-on by naming it under `services` in `verifique.yaml` (see
   [databases](databases.md)).
8. verifique looks for an add-on beside its own program first, where a build leaves the add-ons built
   with it, then in the add-ons folder, then on `PATH`.
9. Each add-on says what kind of thing it provides. Databases are the only kind so far, and naming
   an add-on of another kind under `services` is refused.
10. An add-on speaks a numbered protocol with the core, and the core uses only one that speaks its
    own. One that speaks another is refused, with the command that fetches the matching one.
11. A job that needs an add-on that is not installed ends in an error naming it and the command that
    adds it.
12. An add-on someone else wrote works the same way when it is found and speaks the protocol.
    `plugin update` leaves such add-ons alone.

### Safety

13. The safety promises stay the core's. Database names, the `verifique_` prefix rule and the Redis
    rules in [databases](databases.md) apply whatever add-on does the work, and an add-on refuses a
    name without the prefix.
14. An add-on never installs or starts a server, and keeps passwords out of its errors.

## Never

-   An add-on never weakens the prefix rule.
-   A user never has to install support for a database their projects do not use.

## Limits

-   Upgrading the core any other way than `verifique update` does not upgrade its add-ons: someone
    has to run `plugin update`, which `status` and the panel remind them to do. Until then, an
    add-on of the same protocol keeps working at its older version.
-   Someone upgrading from a version with database support built in has to add the add-ons before
    the next check that needs a database, which otherwise ends in an error naming the command.
-   The Postgres and Redis add-ons do not speak TLS yet (see [databases](databases.md)).

## Open questions

-   **Are other people's add-ons supported?** The core uses any add-on it finds that speaks its
    protocol, and the protocol is documented, but nothing says whether add-ons from outside
    verifique's releases are a supported feature or only possible. Settled by the owner.
