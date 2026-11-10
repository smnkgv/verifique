# Glossary

The words the specs use, and what each one means in verifique. Terms are in alphabetical order. A
spec lists the terms it relies on in its `glossary_terms` header, spelt as they are here.

Two words are easy to confuse. A **database server** is something the user already runs, such as
Postgres, which `verifique.yaml` lists under `services`. **The service** is verifique's own queue
running in the background. The specs never call a database server a service.

**.env**
: The project's own settings file in the repository's main checkout. Every copy of the app in a
slot runs with it, as the user's own copy would, except for the settings `verifique.yaml` sets.
Anything else in it is live. Other files like it can be named under `envFiles`.

**Add-on**
: A separate program, installed on top of verifique with `verifique plugin add <name>`,
that gives it something the core does not carry, such as support for one kind of database. Also
called a plugin.

**Agent check**
: A check in which a fresh coding agent, which did not write the work, uses the running app and
compares it with the user's request. Declared with `type: agent`.

**App**
: One process of a stack that verifique starts and keeps running for a check, such as a web server
or an API, declared under `apps`. An app has a base port and, usually, a ready path.

**Base port**
: The port an app is given in `verifique.yaml`. In a slot, the app runs on the base port plus the
slot number.

**Blocked**
: A verdict saying the check could not get far enough to judge the work, such as when a tool was
refused or a state could not be reached. Sends the branch to needs-you at once.

**Brief**
: The submitting agent's note on how to reach the change, such as "Open Settings, then Filters".
It helps the checker find the change, and never outranks the request.

**Cancelled**
: The end of a job someone stopped with `verifique cancel`. It is not a verdict on the work and
never counts towards needs-you.

**Check**
: One named round of verification a project declares in `verifique.yaml`, such as its test suite
or an agent trying the app. Each submission runs one check against one piece of content.

**Checker**
: Whatever judges the work in a check: the project's command, or the agent in an agent check.

**Command check**
: A check that runs a command, such as a test suite, and judges it by its exit code or by the
verdict it writes.

**Content**
: The exact files of a piece of work, identified by a hash of them, whether committed or not. A
verdict belongs to content, so the same files committed later carry the same verdict.

**Database server**
: A Postgres, MongoDB or Redis server the user already runs. verifique creates and drops databases
on it for each slot, and never installs one. Listed under `services` in `verifique.yaml`.

**Disk floor**
: The free disk below which no check starts, 5 GB unless set otherwise. Below it, verifique deletes
slot copies no check is using.

**Doctor**
: `verifique doctor`, which proves that a project's stacks can run side by side, each with its own
data.

**Error**
: The end of a job whose check broke rather than judged the work: a step failed or timed out, the
checker crashed, or a pass came without proof. It says nothing about the work, so it never counts
towards needs-you.

**Evidence**
: Files a checker saves to show what it saw, such as screenshots or page snapshots. An agent's pass
needs at least one.

**Extra seed**
: One of a stack's named `seeds`, such as a loader for a real dump, which a person can ask for when
starting a stack by hand. Checks never use one.

**Failed**
: A verdict saying the work does not do what was asked, or breaks something that used to work.

**Job**
: One entry in the queue, with a number: a submission of a check, or a started stack.

**Needs-you**
: The state of a branch and check that verifique has stopped accepting, after repeated failures or
one blocked verdict. A person decides what happens next, and `verifique clear` hands it back.

**Panel**
: The small status-bar app that shows the queue at a glance and acts only through the CLI.

**Passed**
: A verdict saying the work does what was asked. For an agent check, it counts only with proof.

**Prepare**
: Steps that install and build the code in a slot before anything runs, such as installing
dependencies.

**Proof**
: What makes an agent's pass count: the steps it took, what it saw, and at least one evidence file.
For a command check, its exit code is the proof.

**Queue**
: The single line of jobs for the whole machine, shared by every repository on it, run as slots
come free.

**Ready path**
: The path verifique requests on an app's port to know it has started. It must prove the app works,
not only that a port is open.

**Replaced**
: The end of a queued job that a newer submission of the same check from the same branch took the
place of. Not a verdict.

**Request**
: The user's original request, word for word, handed over with a submission. The checker judges
the work against it.

**Resource**
: Something limited on the machine that a check needs, such as a phone simulator. Declared on the
machine with a count, and named by a check or stack under `needs`.

**Retry**
: A submission from a branch whose last result for that check was a failure. A retry goes ahead of
that repository's new work.

**Rules**
: The wording `verifique init` proposes for a project's agent instructions, telling agents how to
hand work over and what never to do.

**Seed**
: The project's command that fills a slot's fresh databases with the data a check needs. It runs
for every check, so the data always matches the code.

**The service**
: verifique's queue running in the background, installed per user, starting at logon.

**Setup step**
: An optional command that runs once a stack is ready, such as creating a throwaway user, and whose
output reaches the checker.

**Slot**
: A private place for one check to run: its own copy of the repository, its own ports and its own
databases. The machine has a fixed number of them, two by default.

**Slot number**
: The number that sets a slot apart. Ports and databases are derived from it.

**Snapshot**
: A record of uncommitted work, taken without touching the branch or any file, so it can be checked
as exact content.

**Stack**
: A named description, in `verifique.yaml`, of what a check needs running: how to prepare the
code, how to seed its databases, the settings it gets, and its apps. A stack may have no apps.

**Started stack**
: A stack an agent or a person runs in a slot for a while, to investigate rather than verify. It
gives no verdict and stops by itself.

**Submission**
: Handing work over for checking with `verifique submit`: which check to run, the request, the
brief, and the content.

**Verdict**
: The result of a check on a piece of content: passed, failed or blocked. A check that breaks ends
in an error instead.
