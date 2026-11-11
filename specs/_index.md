# Index

Every spec, listed once. Read [README.md](README.md) for the rules and [GLOSSARY.md](GLOSSARY.md)
for the words. When you add, retire or re-review a spec, update its line here in the same commit.

| Spec | Status | Confidence | What it covers |
|------|--------|------------|----------------|
| [hand-off-and-verdicts](hand-off-and-verdicts.md) | active | best-effort | Handing work over with `submit`, waiting with `wait`, verdicts tied to content, replacing and cancelling, exit codes |
| [slots-and-isolation](slots-and-isolation.md) | active | best-effort | Each check's own copy of the code, own ports and own databases, settings over `.env`, several slots for one check |
| [stacks](stacks.md) | active | best-effort | What a stack is and how it comes up, and starting one by hand with `stack start`, `stack exec`, `stack rebuild` and `stack stop` |
| [checks](checks.md) | active | best-effort | Command checks, agent checks and their instructions, proof and evidence, time limits, needs-you, limited resources |
| [setup](setup.md) | active | best-effort | Installing, the `init` guide and the rules it proposes, and what `doctor` proves |
| [queue](queue.md) | active | best-effort | One queue per machine, the background service, slot count, who goes next, the disk floor, notifications, the panel |
| [databases](databases.md) | active | best-effort | Supported database servers, one set of databases per slot, the `verifique_` prefix rule, Redis numbered databases and slot 0 |
| [safety](safety.md) | active | best-effort | What verifique touches, what is live in `.env`, what leaves the machine, what a checker may do |
| [platforms](platforms.md) | active | best-effort | macOS, Linux and Windows as equals: installing, one command language, stopping steps, the service, notifications, private files |
| [add-ons](add-ons.md) | active | best-effort | Database support as add-ons: `verifique plugin add`, `list`, `update` and `remove`, finding them, the protocol version, safety |
