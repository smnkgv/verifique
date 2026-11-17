use crate::paths::{CONFIG_FILE, NAME};

/// The wording for a project's agent instructions. `init` only ever proposes it: some projects
/// require a person to approve changes to those files.
pub fn rules() -> String {
    format!(
        r#"## Verifying work

Verification goes through {NAME}, so that agents never fight over this machine.

-   Never start the app yourself, even to investigate. To reproduce a bug or try something by hand,
    run `{NAME} stack start --minutes 30`. It waits for a slot and prints the app's addresses and
    logs. Run the project's own commands against it, such as to load data, with
    `{NAME} stack exec <job> -- <command>`. To see your later changes in it, run
    `{NAME} stack rebuild <job>`: it keeps the stack's data, and `--fresh` empties and refills its
    databases, which a change to their structure needs. Run `{NAME} stack stop <job>` as soon as you
    are done, because everyone else is waiting for that slot.
-   Never stop processes by name or pattern, such as `pkill -f`. Other agents' work runs on this
    machine under the same names.
-   To verify work, run:
    `{NAME} submit --check <name> --request-file <file> --brief "<how to reach the change>"`
    Anything after `--` goes on the end of the check's command, such as the tests to run.
-   Put the user's original request in the file word for word. Do not summarise it.
-   Run `{NAME} wait <job>` in the background and carry on, or end your turn. Uncommitted work is
    captured for you, so you do not need to commit first.
-   When it exits non-zero, read the verdict with `{NAME} show <job>`, fix the cause and submit again.
-   Exit 2 means blocked or broken. Exit 3 means a newer submission replaced it or someone cancelled
    it: do not submit it again.
-   After repeated failures a branch goes to `needs-you` and submissions are refused: stop and tell
    the user. Do not move to a new branch to get round it.
-   Call work done only on a pass for its final content."#
    )
}

pub fn guide() -> String {
    format!(
        r#"# Connect this project to {NAME}

You are setting up this repository so that agents can hand verification to {NAME}. {NAME} runs the
app itself, several copies at once, each on its own slot. It gives every slot its own ports and its
own databases. Your job is to describe the project in one file, {CONFIG_FILE}.

Stop and tell the user if a step cannot be done honestly.

Other people's work runs on this machine. Never stop a process by name or pattern, such as
`pkill -f`, which reaches everything that matches. Stop only processes you started, by the id you
got when you started them, and let `{NAME} doctor` run and stop the stacks it proves.

## 1. Check the prerequisites

Database servers must already be running. {NAME} never installs one, and neither should you: tell
the user what is missing and stop. Support for each kind of database is a plugin: `postgres`,
`mongo` and `redis` so far. `{NAME} plugin list` shows which are installed. If the project needs one
that isn't, ask the user to run `{NAME} plugin add <name>`.

## 2. List everything the project verifies

List every app the project ships, such as a website, an API, a phone app or a desktop app, and every
way it verifies work on them today, such as its test suites, each platform's end-to-end tests and
any manual pass its instructions describe. Its CI workflows, scripts and agent instructions show
them. Each one becomes a check, so that no agent has a reason to verify work outside {NAME}.

## 3. Find what two copies of the app would fight over

Read the code and list everything that would collide: ports, databases, cache and queue names,
uploads, temporary folders, scheduled jobs. Find the exact settings the app reads for each one.

## 4. Write {CONFIG_FILE}

Copy the shape of this and keep it short. The app runs as a production build rather than a dev server:
it starts faster, and uses far less memory across several copies. Use the exact command production
uses, which a Dockerfile or deploy config usually shows, not a generic `start` script: some builds
only work one way, and a warning at startup can mean a page will fail later.

```yaml
services:                       # servers that are already running
    postgres: {{ url: postgres://postgres:postgres@127.0.0.1:5432 }}
    redis: {{}}

stacks:
    web:
        prepare:                # installs and builds, once per check
            - pnpm install --frozen-lockfile
            - pnpm build
        seed: pnpm db:migrate && pnpm db:seed   # runs on a fresh, empty database for every check
        seeds:                  # optional: extra data a person can ask for when starting it by hand
            dump: ./scripts/load-dump.sh
        setup: ./scripts/sign-in.sh             # optional: prints sign-in details for a throwaway user
        env:                    # for every step; wins over .env
            DATABASE_URL: "{{postgres.url}}"
            REDIS_URL: "{{redis.url}}"
        apps:
            api:
                cwd: apps/api
                run: node dist/main.js
                port: 3600      # base; each slot adds its number
                ready: /health  # must prove the app works, not just that a port is open
                env:
                    PORT: "{{port}}"
            web:
                cwd: apps/web
                run: pnpm start
                port: 3500
                ready: /
                env:
                    PORT: "{{port}}"
                    API_URL: "{{app.api.url}}"

checks:
    tests:
        prepare:                # a check with no stack starts from a clean checkout
            - npm ci
        run: npm test
    e2e:
        stack: web
        slots: 2                # one stack per worker, sharing one build
        run: npx playwright test
    manual:
        type: agent
        stack: web
        agent: claude           # or {{ command: "your-agent --prompt-file $VERIFIQUE_PROMPT" }}
        tools: []               # added to the user's own agent settings
        notes: |
            What a newcomer would need to know to reach the app.
```

Every command is POSIX `sh`, on every OS: on Windows, {NAME} runs it with the `sh` that comes with
Git for Windows. Write paths with forward slashes, and keep commands to what `sh` and the project's
own tools can do, so the file works for everyone on the project, whatever their OS.

When an agent submits, anything it adds after `--` goes on the end of the check's `run`, so a
check that runs tests can be pointed at some of them. Never make a check pick its tests out of the
brief or the request: they are prose, and a test named there to be left out would run.

A check with `slots` gets one copy of its stack per worker. `VERIFIQUE_STACKS` names a JSON file
listing every copy's slot, host, ports and values, and `VERIFIQUE_SLOTS` lists their slot numbers:
have the suite hand each worker one copy.

Every step has a time limit: 30 minutes for `prepare` and for the check, less for the rest. If a run
the project's agents submit can take longer, such as a whole suite, raise it under `timeouts` at the
top of {CONFIG_FILE}, in seconds, such as `timeouts: {{ runSec: 5400 }}`. The others are
`prepareSec`, `seedSec`, `readySec` and `setupSec`.

A stack needs no apps. A check that only needs its own databases, such as a test suite, gets them
from a stack with just `prepare`, `seed` and `env`, without building the app. With nothing running
before the check, `doctor` takes its settings as proof for a database its seed leaves alone.

verifique reaches apps at `127.0.0.1`. If an app only works at one name, such as `localhost` for
its cookies or allowed origins, set `host` on the stack: every address verifique gives out, to the
apps, the checker and people, then uses it.

Values you can use: `{{slot}}`, `{{port}}`, `{{port.<app>}}`, `{{app.<app>.url}}`, and for each service
`.url`, `.server`, `.database`, `.host`, `.port`, `.user` and `.password`. Redis adds `.db` and
`.prefix`: use `.url` if the app takes a database number, or `.prefix` if it prefixes its keys.

The `.env` in the repository's main checkout is copied into every stack's slot, even for work
submitted from a worktree, and loaded for every step and app, so a production start command needs no
dotenv of its own. A check without a stack gets none, since it has no databases of its own: give a
check that needs `.env` a stack with no apps. If it lacks something the app needs, tell the user what to add rather than filling it in
here. Everything in {CONFIG_FILE} wins over it. Add other gitignored files under `envFiles`. Every
setting that names a port, a database or a shared resource must be overridden here, or two copies
will collide or a check will reach the real thing.

Some tools pass the commands they run only the environment variables they are told to: let
{NAME}'s settings through in that tool's own config, or those commands never see them.

Settings that do not name a resource can still change what the app does. Set the time zone production
uses, and anything else production sets that affects behaviour. Cloud SDKs read credentials from the
home directory as well as `.env`, so a check can reach a real account even with a clean `.env`:
unless a check needs them, point those at nothing, for example `AWS_SHARED_CREDENTIALS_FILE: /dev/null`,
`AWS_CONFIG_FILE: /dev/null` and `AWS_EC2_METADATA_DISABLED: "true"`.

A person starting a stack by hand sometimes wants real data in it. If the project has a way to load
a dump, name it under `seeds`, loading into the slot's own databases through the values above:
`{NAME} stack start --seed dump -- <files>` runs it after `seed`, with the files on the end. Checks
never use it, so their verdicts stay reproducible. A seed runs before the apps start; a command that
needs them up runs with `{NAME} stack exec <job> -- <command>` once the stack is started, in its
slot and with its settings.

Whatever `setup` prints reaches the checker agent and its model provider, so only print throwaway
credentials. Make sure they work as printed, such as an account that can already sign in, because
the checker starts from nothing else.

The queue runs every step with the PATH recorded when it was installed, which `{NAME} service env`
shows, and `doctor` uses the same. Make sure that gives the versions production uses, such as the
same Node, and tell the user if it doesn't. A step must change nothing outside the slot: some
installers clean shared caches, such as `playwright install`, which deletes browser versions other
projects use unless `PLAYWRIGHT_SKIP_BROWSER_GC=1` is set.

A check runs whatever the project's own tests run, including a device, a simulator or a helper they
start, since {NAME} starts only a stack's apps and a check's commands. Name anything the machine has
only a few of under `needs` on the check, with how many it takes, such as `needs: {{ simulator: 1 }}`:
the queue then runs only as many at once as it was started with, such as `--resource simulator=1`.
Whatever runs outside the stack must reach the slot the way the project's own tests reach it, and
`doctor` proves only the stack, so prove it with a real check.

If a check needs the queue started with an option it lacks, such as a resource, ask the user to add
it to `{NAME} service install`, along with the options the queue already has, and wait for them to
say it's done. Never leave a check out to avoid asking.

## 5. Only if the app hardcodes something

You may make small edits so the app reads a value from the environment, keeping today's value as
the default, such as `process.env.PORT ?? 3000`. Make them on a branch and show the user. Anything
larger is for the user to decide: stop and explain what is needed.

## 6. Prove it

Run `{NAME} doctor`. It starts two copies side by side, runs `setup`, confirms each used its own
database, and stops them. Never weaken a `ready` check to make it pass. Read any warning it gives
about a remote database, and tell the user.

Then submit every check once with `{NAME} submit` and read each verdict. Setup proves the wiring,
not the product, so point each check that runs tests, after `--`, at the fewest tests that between
them reach every app and database its stack provides. Setup is finished only when
everything you listed in step 2 has a check and every check has passed, or its only failures also
happen without {NAME}: report those rather than swapping in a test that passes. Leave something out
only when the user says to, and name it in your report. Every failed check counts toward needs-you,
so don't resubmit one to study a failure: investigate in a started stack, which never counts, and
run the tests there with `{NAME} stack exec`. A started stack keeps the {CONFIG_FILE} it started
with, so start a new one after changing that file.

## 7. Propose the rules

The project's agents need the rules below. Propose them to the user as a change to the project's
agent instructions: write them the way those instructions write their own rules, keeping what each
one means, and put them where the instructions say such rules belong. Change every rule or
reference the setup makes untrue, such as how to start the app or run its tests, and keep any part
of it that still holds. Before removing or narrowing a rule, find everything that still depends on
it, such as tests or tools the project still runs itself. Merge a rule below with any rule the
project keeps on the same subject, so that no two contradict.

{rules}
"#,
        rules = rules()
    )
}
