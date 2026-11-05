use crate::paths::{CONFIG_FILE, NAME};

/// The wording for a project's agent instructions. `init` only ever proposes it: some projects
/// require a person to approve changes to those files.
pub fn rules() -> String {
    format!(
        r#"## Verifying work

Verification goes through {NAME}, so that agents never fight over this machine.

-   Never start the app yourself, even to investigate. To reproduce a bug or try something by hand,
    run `{NAME} hold --minutes 30`. It waits for a slot and prints the app's addresses and logs. Run
    `{NAME} release <job>` as soon as you are done, because everyone else is waiting for that slot.
    A hold has the code as it was when you took it: release it and take a new one to see changes.
-   Never stop processes by name or pattern, such as `pkill -f`. Other agents' work runs on this
    machine under the same names.
-   To verify work, run:
    `{NAME} submit --check <name> --request-file <file> --brief "<how to reach the change>"`
-   Put the user's original request in the file word for word. Do not summarise it.
-   Run `{NAME} wait <job>` in the background and carry on, or end your turn. Uncommitted work is
    captured for you, so you do not need to commit first.
-   When it exits non-zero, read the verdict with `{NAME} show <job>`, fix the cause and submit again.
-   Exit 2 means blocked or broken, and 3 means a newer submission replaced this one.
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
the user what is missing and stop. It supports `postgres`, `mongo` and `redis`, and needs `psql`,
`mongosh` or `redis-cli` to reach them.

## 2. Find what two copies of the app would fight over

Read the code and list everything that would collide: ports, databases, cache and queue names,
uploads, temporary folders, scheduled jobs. Find the exact settings the app reads for each one.

## 3. Write {CONFIG_FILE}

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

Values you can use: `{{slot}}`, `{{port}}`, `{{port.<app>}}`, `{{app.<app>.url}}`, and for each service
`.url`, `.server`, `.database`, `.host`, `.port`, `.user` and `.password`. Redis adds `.db` and
`.prefix`: use `.url` if the app takes a database number, or `.prefix` if it prefixes its keys.

The project's `.env` is copied into each slot and loaded for every step and app, so a production
start command needs no dotenv of its own. Everything in {CONFIG_FILE} wins over it. Add other
gitignored files under `envFiles`. Every setting that names a port, a database or a shared
resource must be overridden here, or two copies will collide or a check will reach the real thing.

Settings that do not name a resource can still change what the app does. Set the time zone production
uses, and anything else production sets that affects behaviour. Cloud SDKs read credentials from the
home directory as well as `.env`, so a check can reach a real account even with a clean `.env`:
unless a check needs them, point those at nothing, for example `AWS_SHARED_CREDENTIALS_FILE: /dev/null`,
`AWS_CONFIG_FILE: /dev/null` and `AWS_EC2_METADATA_DISABLED: "true"`.

Whatever `setup` prints reaches the checker agent and its model provider, so only print throwaway
credentials.

## 4. Only if the app hardcodes something

You may make small edits so the app reads a value from the environment, keeping today's value as
the default, such as `process.env.PORT ?? 3000`. Make them on a branch and show the user. Anything
larger is for the user to decide: stop and explain what is needed.

## 5. Prove it

Run `{NAME} doctor`. It starts two copies side by side, runs `setup`, confirms each used its own
database, and stops them. Setup is finished only when it passes. Never weaken a `ready` check to make it pass.
Read any warning it gives about a remote database, and tell the user.

## 6. Propose the rules

Show the user this text and ask them to add it to the project's agent instructions:

{rules}
"#,
        rules = rules()
    )
}
