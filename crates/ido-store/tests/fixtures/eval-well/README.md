# eval-well

A synthetic personal knowledge well used to score ido's retrieval (keyword-only, semantic-only,
and hybrid) against a fixed query set — see `docs/mcp-server.md` §6.7. The content is a fictional
solo person's notes across five hobby/life areas (a homelab rack build, a sourdough starter log, a
bike restoration, a freelance firmware-review contract, and personal finance). It is **not** about
ido itself, so self-reference can't muddy the eval.

The well mirrors `dev-well/` in shape: `.ido/well.toml`, `notes/` (with subfolders), `wiki/` (with
two organisational folders), `tasks/` (one markdown file per task, frontmatter metadata) and
`tasks/goals/`. Ids follow the same conventions the store uses everywhere else (see
`crates/ido-store/src/search.rs`'s tests and `crates/ido-store/src/wiki.rs`'s module doc):

- a **note**'s id is its `notes/`-relative path, without the `.md` extension (`"homelab/backup-strategy"`)
- a **wiki** page's id is its slug — the file stem — regardless of which folder it physically sits
  in (`"acme-contact"`, not `"people/acme-contact"`)
- a **task** or **goal**'s id is its file stem (`"order-rack-rails"`, `"bike-restoration"`)

## `eval.jsonl`

One JSON object per line (LF line endings), each shaped:

```json
{"query": "...", "expected": [{"kind": "note", "id": "folder/some-note"}], "class": "paraphrase"}
```

- `query` — the natural-language (or, for `identifier` cases, literal-token) search string.
- `expected` — the gold relevant set: one or more `{kind, id}` pairs. `kind` is one of `"note"`,
  `"wiki"`, `"task"`, `"goal"`. Every id here has a backing file in this fixture — round-trip it
  through `get_entry` the same way a real `search` hit would.
- `class` — one of the three below. Most queries have a single expected id; a handful of the
  `identifier` and (by construction) most `cross-section` cases carry 2–3.

## Classes

- **`paraphrase`** — the query shares **no content words** with its target entry's text (title,
  frontmatter title, and body). This is the entire reason semantic search exists: keyword search
  cannot find a note that says "session keys are kept in the OS keychain" when asked "where do we
  store login credentials". Stopwords (articles, prepositions, pronouns, auxiliary verbs, …) may
  coincide; any other word may not, checked token-by-token (case-insensitive, punctuation
  stripped) against the *entire* target file, not just the matching sentence.
- **`identifier`** — a rare, exact token: an invented config key, a part number, a frame serial, an
  account number, an unusual person's name. Only keyword/lexical matching is expected to nail
  these reliably; a good hybrid search must not regress them relative to keyword-only.
- **`cross-section`** — the expected set spans **two or more kinds** (a wiki page + a task, a note
  + a goal, a note + a task + a goal, …) about one topic, proving retrieval isn't scoped to a
  single section.

## Negative control

`tasks/recalibrate-the-oscilloscope.md` (`archived: true`) exists purely to prove **archived tasks
never surface**. Its vocabulary (Tektronix, oscilloscope, graticule, beam-finder, trimmer pots,
CRT, vertical amplifier) is deliberately unique to that one file — it appears in no other entry in
this well and in no `eval.jsonl` query or expected id. If any retrieval mode ever returns it, that
mode is leaking archived content and the run should fail loudly, not just score lower.

## Regenerating or extending

Keep these invariants if you add to this fixture:

- Every `expected` id must resolve to a real file, using the id conventions above.
- A `paraphrase` query must share zero content words with its target's *entire* text — check the
  whole file, not just the sentence you had in mind; a long note's incidental vocabulary is an easy
  way to accidentally leak a word back in.
- The archived task stays archived, its vocabulary stays exclusive to it, and it stays absent from
  every `expected` list.
- Keep the class minimums from `docs/mcp-server.md` §6.7: at least 12 paraphrase, 8 identifier, and
  6 cross-section cases.
