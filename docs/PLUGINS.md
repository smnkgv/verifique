# Plugins

verifique's core knows how to queue, isolate and verify. Everything specific to one technology, such
as giving each slot its own Postgres database, is a plugin: a separate program installed on top of
the core. People install only what their projects use.

```bash
verifique plugin add postgres redis
verifique plugin list
verifique plugin update
verifique plugin remove redis
```

`plugin add` fetches each plugin from the release verifique itself came from, checks it against that
release's `SHA256SUMS`, and puts it in `~/.verifique/plugins` (`%USERPROFILE%\.verifique\plugins` on
Windows). After upgrading verifique, `plugin update` brings the plugins to the same version, and
`plugin list` and `verifique doctor` say when one is behind.

## Finding a plugin

A plugin called `postgres` is the program `verifique-postgres` (`verifique-postgres.exe` on
Windows). verifique looks for it in this order:

1. the plugins folder, or `VERIFIQUE_PLUGINS` when that is set;
2. the folder holding the `verifique` binary, where a build from source leaves its plugins;
3. `PATH`.

A project uses a database plugin by naming it under `services` in `verifique.yaml`:

```yaml
services:
    postgres: { url: postgres://postgres:postgres@127.0.0.1:5432 }
```

## The protocol

verifique runs the plugin with one operation as its only argument, writes a JSON request to its
stdin and reads one JSON reply from its stdout. The plugin exits 0 with the answer, or exits non-zero
with `{"error": "..."}`, a sentence written for the person who will read it. Anything on stderr is
shown only when the reply can't be read.

This is protocol 1. Every request carries `"protocol": 1`, and a plugin refuses a number it doesn't
speak. [plugins/kit](../plugins/kit/src/lib.rs) is the protocol as a Rust crate, which verifique and
its own plugins share.

### describe

No request. verifique asks this first.

```json
{"protocol": 1, "name": "postgres", "version": "0.3.0", "provides": "database",
 "default_url": "postgres://127.0.0.1:5432"}
```

`provides` says what kind of plugin this is. `database` is the only kind so far. `default_url` is
where the server usually is, for a project that names none.

### Database operations

Each is about one slot's share of a server that is already running:

```json
{"protocol": 1, "server": "postgres://postgres:postgres@127.0.0.1:5432",
 "database": "verifique_shop_s4", "slot": 4}
```

| Operation  | Does                                                    | Replies                          |
|------------|---------------------------------------------------------|----------------------------------|
| `settings` | Names what the slot's apps need to reach its database   | `{"settings": {"url": "...", ...}}` |
| `reset`    | Starts the slot from nothing, before every check        | `{}`                             |
| `remove`   | Drops whatever the slot holds                           | `{}`                             |
| `in-use`   | Says whether anything has reached the slot's database   | `{"in_use": true}`               |

Each setting becomes `{<service>.<name>}` in `verifique.yaml`, so `url` from the postgres plugin is
`{postgres.url}`. `url` and `database` are expected. `server`, `host`, `port`, `user` and `password`
are customary.

`in-use` is how `verifique doctor` proves each slot used its own data: an app that ignored its
settings and wrote to another database leaves its slot's database empty.

### Rules

- **Only ever touch databases named `verifique_*`.** verifique names every slot's database with
  that prefix, and a plugin refuses any other name, so a slot can never be a real database.
- **Never install or start a server.** A running server is a prerequisite, and an error says so.
- **Keep passwords out of errors.** Client libraries often echo the URL they were given.
