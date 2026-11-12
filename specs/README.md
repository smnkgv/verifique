# Product specs

These specs say what verifique does and why, in the language of the people who use it: developers
who run many coding agents on one machine, and the agents themselves. They are not a user guide
(that is the repository's `README.md`) and not a record of how it is built (that is
`docs/DESIGN.md` and the code). A spec is the answer to "what should happen, and why?", written so
that it would still be true if verifique were rewritten from scratch.

Start with [`_index.md`](_index.md) for the list of specs, and [`GLOSSARY.md`](GLOSSARY.md) for the
words they use.

## Rules

1. **A spec knows nothing about the implementation.** No file, module, flag-parsing, crate or
   storage-schema names, and no names of the OS mechanisms underneath. Test every sentence: would it
   survive a rewrite in another language unchanged? If not, it belongs in the code or in
   `docs/DESIGN.md`.
2. **The product's own interface may appear.** CLI commands and their options, `verifique.yaml`
   keys, the values and environment variables a project's commands receive, the verdict a check
   writes, and the exit codes an agent acts on are what users type and read, so they are part of
   what the product is. Name them exactly.
3. **Organise by product area, never by the code.** A spec covers something a user can name, such
   as "started stacks" or "the queue", even when the behaviour is spread across the code. Two specs
   never describe the same behaviour: one owns it and the other links to it.
4. **Say what isn't known.** When the sources disagree, or nobody has decided, write an open
   question in the spec's "Open questions" section rather than inventing an answer. Each question
   says what is known, where the sources differ, and what would settle it.
5. **Retire, don't delete.** When behaviour goes away, set the spec's status to `deprecated` and
   say what replaced it, if anything. The file stays, so a link to it still explains what used to
   happen.
6. **A behaviour change updates its spec in the same commit.** A change that makes a spec untrue,
   or adds behaviour no spec covers, is not finished until the spec says so.
7. **Every spec starts with the header below** and is listed exactly once in
   [`_index.md`](_index.md). New specs start from [`TEMPLATE.md`](TEMPLATE.md).
8. **Words mean what the glossary says.** A term a spec relies on is in
   [`GLOSSARY.md`](GLOSSARY.md) and in the spec's `glossary_terms`. Add a term before using it with
   a new meaning.
9. **No names of private clients, platforms or projects.** The repository is public. Examples use
   made-up apps.

## The header

```yaml
---
status: draft | active | deprecated
last-reviewed: YYYY-MM-DD
owner: Hlib Semeniuk
confidence: authoritative | best-effort | unverified
related: [other-spec-slug, another-slug]
glossary_terms: [Slot, Stack]
---
```

-   **status**
    -   `draft`: decided or proposed, but not what the product does today. A draft says so at the top.
    -   `active`: what the product does today.
    -   `deprecated`: what the product used to do. Says what replaced it.
-   **last-reviewed**: the last day someone read the whole spec against the product and the owner's
    intent, not the last day the file changed.
-   **owner**: who decides what the spec should say.
-   **confidence**: how far a reader can rely on it.
    -   `authoritative`: the owner has confirmed it, statement by statement.
    -   `best-effort`: written from the owner's documents and checked against the product's
        behaviour, with every known gap recorded as an open question. Not yet confirmed by the
        owner.
    -   `unverified`: recorded second-hand, or describes something not built, so nobody has checked
        it against the product.
-   **related**: slugs of other specs a reader of this one will need. A slug is a file name without
    `.md`.
-   **glossary_terms**: the glossary entries this spec relies on, spelt as the glossary spells them.

## Where the specs came from

The first versions were written on 2026-11-09 from the user guide, `docs/DESIGN.md` and the
project's web page, with the code read only to confirm behaviour. Where those sources disagreed,
the spec records an open question.
