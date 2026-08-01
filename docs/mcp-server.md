# ido MCP server — design + implementation roadmap

How ido's well becomes a **second brain that agents can query**: a local MCP server over the
notes / wiki / tasks store, read-only first, with **semantic search** as the headline capability.

This is the working design doc. The ecosystem-level rationale (why MCP, why ido first, stdio
before remote) lives in [`vendor/latent-design/docs/knowledge-architecture.md`](../vendor/latent-design/docs/knowledge-architecture.md)
§ "How Phase 0 fits ido" — **read that first; this document does not restate it.** What follows
is the concrete plan for ido: the crate split, the protocol baseline, the tool surface, and the
retrieval stack.

**TL;DR:** Split the store out of `src-tauri` into a tauri-free `ido-store` crate, put a second
binary (`ido-mcp`, [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) + stdio) beside it,
and ship **seven read-only tools**. Semantic search is a separate, additive layer: chunk markdown
→ embed locally with [fastembed](https://docs.rs/fastembed) → **brute-force cosine over an
in-memory f32 matrix** (no ANN index — a personal well is far too small to need one) → fuse with
the existing lexical scan via reciprocal rank fusion. Writes come last, gated, and only after the
read path is trusted.

---

## 1. Goals and non-goals

**Goals**

1. Any MCP client (Claude Code first) can **read and search** the well without ido running.
2. **Semantic search** — "what did I decide about auth token storage?" finds the note even when
   it never uses the word "auth".
3. **One implementation of the store.** The MCP server and the Tauri backend share data-access
   code; there is never a second, drifting copy of "how a task file is parsed".
4. **Local-first, offline, no network.** No embedding API, no telemetry, no cloud index. A well
   is the user's private notes; nothing leaves the machine.
5. Read-only by default. Writes are an explicit, later, opt-in phase.

**Non-goals (for this document)**

- **ido as an MCP *client*** — in-app AI calling out to external MCP tools. Separate concern,
  separate doc.
- **Remote / hosted MCP** (Streamable HTTP + OAuth). Phase 2 in the ecosystem doc; only on a
  concrete second-client trigger. The tool code is transport-agnostic, so this is not thrown away.
- **A local LLM in-app.** The embedding model here is a 100–300 MB retrieval model, not a
  generative one. The Gemma/on-device-assistant ambition in [`CLAUDE.md`](../CLAUDE.md) is
  adjacent and shares the ONNX/model-cache plumbing, but is out of scope.
- **Notes as `[[link]]` targets** and **wiki folders** — still deliberate non-goals of the store.

---

## 2. What already exists (and maps almost 1:1)

The store is closer to an MCP server than it looks. Every tool below has a backing function today:

| Proposed MCP surface | Backed by | File |
|---|---|---|
| `search` (lexical half) | `search::search` — cross-section brute scan, relevance-ranked | [`src-tauri/src/search.rs`](../src-tauri/src/search.rs) |
| `get_entry` | `notes::read_note` / `wiki::read_page` / `tasks::list_tasks` | `notes.rs`, `wiki.rs`, `tasks.rs` |
| `list_entries` | `notes::list_tree`, `wiki::list_wiki` | `notes.rs`, `wiki.rs` |
| `backlinks` | `wiki::backlinks` — already **cross-section** (notes + wiki + task/goal bodies) | `wiki.rs` |
| `list_tasks` / `list_goals` | `tasks::list_tasks` / `tasks::list_goals` | `tasks.rs` |
| `well_info` | `wells` manifest + `tasks::task_columns` | `wells.rs` |

`search.rs` even says so in its module doc: *"The same scan is the natural backing for a future
MCP `search` tool."* Good — that intent holds.

Two structural facts that make the split cheap:

- **Commands take plain args and return plain values.** `pub fn read_page(well: String, slug:
  String) -> String`. The `#[tauri::command]` attribute is additive; the function underneath is an
  ordinary Rust fn, already unit-tested with tempdirs and no mock runtime.
- **Only three functions touch Tauri types at all**: `wells::pick_folder`, `wells::open_well`,
  `wells::create_well` (they take `tauri::AppHandle` for the dialog plugin and the recent-wells
  registry). Plus `window` and `external`, which are UI-only by definition.

So the tauri-free surface is ~90% of the backend already.

---

## 3. Architecture — a three-crate workspace

### 3.1 Target layout

```
ido/
  Cargo.toml            # workspace root + the `ido-ui` frontend package (unchanged)
  crates/
    ido-store/          # NEW — the store. No tauri, no wasm. Pure std::fs + serde.
      src/{lib,model,paths,frontmatter,notes,wiki,tasks,repeat,assets,search,wells,session}.rs
      src/index/        # NEW (phase 2) — chunking, embedding, vector index, hybrid fusion
    ido-mcp/            # NEW — the MCP server binary (rmcp + stdio)
      src/{main,server,tools,resources,render}.rs
  src-tauri/            # SHRINKS — thin #[tauri::command] wrappers + window/external/registry
    src/{lib,window,external,registry,commands}.rs
```

### 3.2 Why a separate crate and not "just link `ido_lib`"

`src-tauri` is a `staticlib`/`cdylib`/`rlib` crate that depends on `tauri`. Linking it from a
plain CLI binary drags the whole Tauri runtime (and on Linux, webkit system deps) into a program
that only needs to read markdown files. That breaks the "a weekend afternoon for the skeleton"
premise from the ecosystem doc, and makes CI for the MCP binary heavier than CI for the app.

The extraction is mechanical:

1. `git mv` the pure modules into `crates/ido-store/src/`, drop the `#[tauri::command]`
   attributes, keep every signature and every test verbatim.
2. `src-tauri/src/commands.rs` becomes a wall of one-line re-exports:
   ```rust
   #[tauri::command]
   pub fn read_page(well: String, slug: String) -> String {
       ido_store::wiki::read_page(&well, &slug)
   }
   ```
   `generate_handler!` points at those. **The frontend `ipc` layer does not change at all** — same
   command names, same arg shapes, same camelCase→snake_case rule.
3. `wells::open_well` / `create_well` split: the scaffold/migrate logic moves to the store; the
   `AppHandle`-touching registry write stays in `src-tauri`.
4. `just test` grows a second target: `cargo test -p ido-store` becomes the real test suite,
   `cargo test -p ido` keeps whatever is left.

**Risk:** this is a large, boring, high-churn diff across ~4,300 lines of backend. Do it as its
own commit with **zero behaviour change**, verified by the existing test suite passing unmoved.
Do not combine it with the MCP work.

### 3.3 Process model

```
        ┌───────────────────────────┐
        │  ido.app (Tauri)          │
        │  frontend ⇄ #[command]s   │──┐
        └───────────────────────────┘  │
                                       ├──▶  crates/ido-store  ──▶  <well>/  (markdown on disk)
        ┌───────────────────────────┐  │                            <well>/.ido/index/
        │  ido-mcp (stdio, spawned  │──┘
        │  by the MCP client)       │
        └───────────────────────────┘
```

Two processes, one store, **no IPC between them**. The filesystem is the interface — which is the
whole point of a local-first markdown app. Concurrency is handled the way the app already handles
it: last-write-wins on whole files, plus a lockfile around index rebuilds (§6.6).

### 3.4 Which well?

`ido-mcp --well <path>`, resolved in this order:

1. `--well <path>` flag.
2. `IDO_WELL` env var.
3. The most-recent entry in `<app_data_dir>/wells.json` (the registry the launcher already keeps).

**One well per server process.** Multi-well is a `well` parameter on every tool plus a
`list_wells` tool — cheap to add later, but it doubles the ambiguity surface for the model on day
one ("which well did you mean?"), and MCP clients can trivially register two servers with
different `--well` values instead. Start pinned.

---

## 4. Protocol baseline

### 4.1 The spec moved, recently

MCP's current revision is **2026-07-28** — the largest change since launch, released four days
before this document. What matters for a local stdio server:

| Change | Impact on ido-mcp |
|---|---|
| **Stateless core** — `initialize`/`notifications/initialized` handshake removed; protocol version + client capabilities ride in `_meta` on every request | None if we use the SDK; do **not** hand-roll a handshake |
| **`server/discover` is mandatory** — servers MUST advertise supported versions, capabilities, identity | SDK-provided; verify it responds |
| **`resultType` required on every result** (`"complete"` / `"input_required"`) | SDK-provided |
| **`CacheableResult`** — `ttlMs` + `cacheScope` now **required** on `tools/list`, `resources/list`, `prompts/list`, `resources/read`, `resources/templates/list` | We must pick values. Tools/prompts: long TTL, `cacheScope: "private"`. `resources/read`: short TTL — a note can change under the client at any moment |
| `tools/list` **SHOULD be deterministically ordered** | Emit tools in a fixed order; helps client-side prompt caching |
| `resources/subscribe` → `subscriptions/listen` | Skip entirely. A stdio server for a local file store does not need change-push; clients re-read |
| **Roots, Sampling, Logging deprecated** | Don't adopt any. **Log to stderr** — never stdout, which is the JSON-RPC channel |
| Tasks moved to an official extension | Not needed |
| MRTR replaces server-initiated requests (`elicitation/create` etc.) | Only relevant if write tools ever want confirmation (§8) |

### 4.2 SDK

**[`rmcp`](https://crates.io/crates/rmcp) 3.0.0-beta.2** (2026-07-24) implements the stable
2026-07-28 spec with back-compat to 2025-11-25 and earlier. Features needed:

```toml
rmcp = { version = "3.0.0-beta.2", features = ["server", "macros", "transport-io", "schemars"] }
```

Shape (from the SDK README; `#[tool_router(server_handler)]` generates both the router and the
`ServerHandler` impl):

```rust
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router,
           ServerHandler, ServiceExt, transport::stdio};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// What to look for. Natural language works — this is semantic + keyword search.
    query: String,
    /// Restrict to one section: "notes" | "wiki" | "tasks". Omit to search everything.
    section: Option<String>,
    /// Max hits (default 10, cap 50).
    limit: Option<u32>,
}

#[derive(Clone)]
struct Ido { well: PathBuf, index: Arc<RwLock<Index>> }

#[tool_router(server_handler)]
impl Ido {
    #[tool(description = "Search the well's notes, wiki pages, and tasks. …")]
    async fn search(&self, Parameters(p): Parameters<SearchParams>) -> String { … }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Ido::open(well)?.serve(stdio()).await?.waiting().await?;
    Ok(())
}
```

⚠️ **`3.0.0-beta.2` is a beta.** Pin it exactly, and expect one churn pass before 3.0 final. This
is the cost of tracking a spec that shipped last week; the alternative (targeting 2025-11-25) means
a migration later anyway.

`tokio` is already a `src-tauri` dependency (`features = ["sync"]`); `ido-mcp` needs
`features = ["rt-multi-thread", "macros", "io-std"]`.

---

## 5. The tool surface

### 5.1 Design rules

Straight from Anthropic's [tool-writing guidance](https://www.anthropic.com/engineering/writing-tools-for-agents),
and they matter more than the schemas:

- **Few, high-impact tools.** Seven, not thirty. Overlapping tools make the model pick badly.
  Claude Code switches to a lightweight tool-search index once tool definitions exceed ~10% of
  context — a small, well-described surface stays loaded.
- **Bound every response.** Claude Code caps tool results at ~25,000 tokens. Every tool takes
  `limit` / `offset` / `max_chars` with sane defaults and hard caps, and says so in its
  description.
- **Descriptions are the interface.** Be prescriptive about *when* to call, not just what it does.
  Recent models under-reach for tools whose descriptions only describe.
- **Return ids that round-trip.** A `search` hit's `id` must be exactly what `get_entry` accepts.
- **Encourage many small searches** over one broad one — say it in the `search` description.

Names are **not** prefixed with `ido_`: MCP clients already namespace by server (Claude Code shows
them as `mcp__ido__search`), so a prefix is redundant tokens on every definition.

### 5.2 Phase-1 tools (read-only)

| Tool | Input | Returns |
|---|---|---|
| `search` | `query`, `section?`, `mode?` (`hybrid`\|`semantic`\|`keyword`, default `hybrid`), `limit?` (10, cap 50) | Ranked hits: `kind`, `id`, `title`, `snippet` (~300 chars, match-centred), `score`, `heading_path` |
| `get_entry` | `kind` (`note`\|`wiki`\|`task`\|`goal`), `id`, `max_chars?` (8000), `offset?` | Full markdown + parsed frontmatter + created/modified + inbound-link count. Truncation is explicit and tells the model how to page |
| `list_entries` | `section`, `prefix?` (folder path, notes only), `limit?` (100), `offset?` | `{id, title, modified, size}` — the cheap map of the well, for orientation before searching |
| `backlinks` | `slug` (wiki page) | Cross-section `{kind, id, title}` of everything linking here |
| `list_tasks` | `status?`, `tag?`, `goal?`, `due_before?`, `due_after?`, `include_archived?` (false), `limit?` | Task rows: id, title, status, priority, tags, due, goal, checklist rollup, completed |
| `list_goals` | `include_archived?` | Goals + derived progress (done/total, from the linked tasks) |
| `well_info` | — | Well name/path, enabled sections, board columns, entry counts per section, index status (model, chunk count, last build, staleness) |

`well_info` is deliberately first-class: it is the cheapest possible orientation call, and it tells
the model whether semantic search is actually available in this well or whether it's getting
keyword-only results.

### 5.3 Resources

Tools do the work; resources make entries **addressable**. Register resource *templates* so any
entry can be referenced without enumerating everything:

```
ido://note/{path}     ido://wiki/{slug}     ido://task/{id}     ido://goal/{id}
```

`resources/list` returns a **bounded** set — the N most-recently-modified entries, not the whole
well (a 5,000-note well would blow the client's context on a list call). `resources/read` maps
onto the same code path as `get_entry`, with a short `ttlMs`.

### 5.4 Prompts (phase 4, optional)

`daily_review` (overdue + due-today + recently touched notes), `weekly_digest` (what changed,
which goals moved). Cheap to add once the tools exist; skip until the read path is boring.

### 5.5 Response shaping

Return **markdown text** as the primary content, with `structuredContent` + an `outputSchema`
alongside for clients that want it (2026-07-28 loosened `structuredContent` to any JSON value).
Markdown is what the model reads best and what a human debugging the server can eyeball.

Snippets: centre on the match, respect char boundaries (the existing `search::snippet` already
does char-safe truncation), and prefix with the heading path (`Architecture › Storage › …`) so a
chunk is legible out of context.

---

## 6. Semantic search

The current search is `haystack.to_lowercase().contains(needle)`. It is fast, dependency-free, and
**cannot find a note that says "session keys are kept in the OS keychain" when you ask about
"where do we store credentials"**. That gap is the entire reason this section exists.

### 6.1 Pipeline

```
markdown file ──▶ chunk ──▶ embed ──▶ vectors.bin ┐
                                                  ├──▶ cosine top-k ┐
query ─────────▶ embed ───────────────────────────┘                 ├──▶ RRF ──▶ ranked hits
query ─────────▶ existing lexical scan ──────────────▶ top-k ───────┘
```

Every stage is local. Every stage is optional-degradable: if the index is missing or the model
isn't downloaded, `search` falls back to keyword mode and says so in `well_info`.

### 6.2 Chunking

Embedding a whole 4,000-word note produces one mushy vector that matches everything weakly.
Embedding every line produces context-free fragments. The useful unit for markdown is a
**heading-scoped section**, capped:

- Segment with `pulldown-cmark` (**already a backend dependency**) on heading events, then split
  any section over ~450 tokens at paragraph boundaries, with ~15% overlap between splits.
- **Prefix every chunk with its heading path and entry title** before embedding:
  `"Auth rewrite › Storage › Session tokens\n\n<body>"`. This is the single highest-leverage
  trick in the whole pipeline — it gives an isolated chunk the context the reader had.
- Tasks and goals: frontmatter `title` + `tags` + body as one chunk (they're short), plus the
  `goal:` title when linked.
- **Never chunk across a fenced code block.** (Same class of bug as the `- [ ]`-in-a-code-fence
  issue `tasks::count_checks` already avoids by using parser events rather than line scanning —
  reuse that instinct.)
- Skip: `assets/` (binary), frontmatter key-value noise other than title/tags.

Store per chunk: `{chunk_id, kind, entry_id, heading_path, byte_range, text}` so a hit can point
at an exact place in the source file, and `get_entry` can be called with the right `offset`.

### 6.3 Embedding model

**[`fastembed`](https://docs.rs/fastembed) 5.17.x** — synchronous, no tokio requirement, ONNX
Runtime under the hood, download-once/run-offline, and it exposes `UserDefinedEmbeddingModel` for
fully-offline bring-your-own-ONNX.

| Model | Dim | Disk | Notes |
|---|---|---|---|
| **`BGESmallENV15`** (fastembed default) | 384 | ~130 MB | **Recommended start.** Strong retrieval quality per byte, English-first, tiny vectors |
| `AllMiniLML6V2` | 384 | ~90 MB | Smallest, slightly weaker. Fallback for constrained machines |
| **`EmbeddingGemma300M`** (`…Q4` 4-bit build available) | 768 (Matryoshka → 512/256/128) | <200 MB quantized | **Recommended upgrade.** Multilingual, on-device-designed, truncatable dims. Also the obvious bridge to the Gemma ambition in `CLAUDE.md` |

Ship `BGESmallENV15` as the default and put the model choice in `.ido/well.toml` (or app settings)
— the index manifest records which model built it, so switching triggers a rebuild rather than
silently mixing vector spaces.

**Distribution is the real problem, not quality.** Three unknowns to resolve in a spike before
committing (§10):

1. **ONNX Runtime shared library.** fastembed depends on `ort` 2.0.0-rc.13. `ort-download-binaries`
   fetches it at build time; for a shipped Tauri app the runtime `.dylib`/`.dll`/`.so` must be
   bundled and found at load time. This is the single biggest packaging risk in the whole plan.
2. **Model download.** ~130 MB on first use, from HuggingFace. Cache in `<app_data_dir>/models/`
   (via `TextInitOptions::with_cache_dir`), **not** in `.ido/` — models are per-machine and shared
   across wells; `.ido/` is per-well rebuildable cache.
3. **Binary size and cold start.** Measure `ido-mcp` startup with the model loaded; the client
   spawns it per session.

**Fallback if (1) proves ugly:** make the semantic layer a **compile-time feature** (`--features
semantic`) and ship keyword-only by default, with semantic as an opt-in the user enables in
settings (which then downloads runtime + model). `well_info` reports which mode is live. This
keeps the MCP server shippable while the packaging is sorted.

### 6.4 Vector storage — brute force, and why that's the right answer

Napkin math for a *large* personal well — 2,000 notes averaging 1.5 KB, ~5,000 chunks at 384 dims:

| Metric | Value |
|---|---|
| Index size (f32) | 5,000 × 384 × 4 B = **7.7 MB** |
| One query | 5,000 × 384 = 1.9M multiply-adds → **well under 1 ms** |
| At 100,000 chunks (a 40× bigger well) | 154 MB, ~40M MACs → **single-digit ms** |
| Full index build (CPU) | ~20–60 s one-time; incremental rebuilds negligible |

Published 2026 benchmarks put exact brute-force cosine at ~27 QPS on 45k × **1024**-dim vectors —
and that's the number for a *slow* exact implementation on vectors 2.7× wider than ours. A well
would need to be two orders of magnitude larger than any plausible personal knowledge base before
HNSW earns its complexity, index-build time, and recall loss (usearch/hnswlib land at 0.987–0.995
recall@10 — worse than the 1.000 we get for free).

**So: no `sqlite-vec`, no `usearch`, no `hnsw_rs`.** A `Vec<f32>` matrix, `mmap`ed or read into
memory at startup, and a hand-written dot-product sweep. This is exactly the same call `search.rs`
already made ("Wells are small and local, so a scan is fast") and it's still right.

Revisit only if a real well crosses **~100k chunks**; `well_info` reports chunk count, so the
trigger is observable rather than theoretical.

On-disk format under `<well>/.ido/index/`:

```
manifest.json    schema version, model id, dim, per-file {sha256, mtime, chunk range}
vectors.bin      raw little-endian f32, row-major, chunk-id ordered
chunks.jsonl     {chunk_id, kind, entry_id, heading_path, byte_range, text}
```

Optional later: **int8 scalar quantization** — 4× smaller, ~1% recall cost. Not needed at these
sizes; note it and move on.

### 6.5 Hybrid retrieval

Neither half is sufficient. Keyword nails exact identifiers (`archive_done_after_days`, a person's
name, a slug); vectors nail paraphrase. Fuse with **reciprocal rank fusion**:

```
score(d) = Σ_lists  1 / (k + rank_list(d))       k = 60
```

Take top-50 from each list, fuse, return top-`limit`. `k = 60` is the paper's constant and holds up
across corpora — **don't tune it**; any gain is smaller than the noise in a hand-built eval set.

RRF needs no score normalization (it uses ranks, not scores), which is exactly why it fits here:
the lexical scorer produces `in_title * 1000 + occurrences`, which is not comparable to cosine
similarity in any principled way.

**Reranking** — fastembed's `TextRerank` cross-encoder over the fused top-20 — is a real quality
lever but adds a second model, more latency, and more packaging. Phase 4, measured against the
eval set, dropped if it doesn't pay.

### 6.6 Freshness and incremental indexing

The app and the MCP server both write; the index must not go stale silently.

- **Manifest-driven.** Each entry records `{sha256, mtime, size, chunk_range}`. A sweep stats every
  markdown file (a few thousand `stat` calls — single-digit ms) and re-embeds only files whose
  mtime *and* hash changed.
- **When:** on `ido-mcp` startup, and before any `search` call if >N seconds since the last sweep
  (debounced, mirroring how `search_gen` debounces the palette).
- **Who rebuilds:** either process. Both take an advisory lock (`.ido/index/.lock`, stale-lock
  timeout) around a rebuild; a reader with a stale index answers from what it has and reports
  `stale: true` in `well_info` rather than blocking.
- **App-side rebuild** is the nice path: ido already knows exactly which file it just wrote, so it
  can re-embed one entry on save. Wire this when the app grows an in-app semantic search — until
  then the MCP server's sweep is sufficient and simpler.
- **Never index `.ido/` or `assets/`.** Never let a symlink walk out of the well.

### 6.7 Evaluating it

Without an eval, "semantic search" is vibes. Build the smallest useful harness:

- `crates/ido-store/tests/fixtures/eval-well/` — 40–60 synthetic notes/pages/tasks.
- `eval.jsonl` — ~30 `{query, expected_entry_ids}` pairs, written to include paraphrase cases
  (query and target share no content words), exact-identifier cases, and cross-section cases.
- Report **recall@5** and **MRR** for keyword-only / semantic-only / hybrid.
- Gate: hybrid must beat keyword-only on recall@5 and must not regress the exact-identifier
  queries. If it doesn't, the chunking is wrong — fix that before touching models or `k`.

---

## 7. Phases

Each phase is independently shippable and independently useful.

### P0 — Extract `ido-store` *(no new features)*

Move the pure modules out of `src-tauri`; `src-tauri` becomes command wrappers. Frontend untouched.
**Done when:** every existing backend test passes unmoved, `just verify` is green, and the app
behaves identically.

### P1 — Read-only MCP server, keyword search

`crates/ido-mcp` with rmcp + stdio and the seven tools of §5.2, `search` backed by the existing
lexical scan. Registered in `.mcp.json`, driven from Claude Code against a real well.
**Done when:** an agent can answer "what's on my board and what did I write about X" without ido
running; tool results stay under budget on a large well; a `just mcp` recipe runs it.

### P2 — Semantic index

Chunker, fastembed integration, `vectors.bin` + manifest, brute-force cosine, RRF fusion, the
eval harness. `mode` parameter on `search`; `well_info` reports index state. Graceful degradation
to keyword when the model/runtime is absent.
**Done when:** hybrid beats keyword on the eval set, a cold index builds in under a minute on a
2,000-note well, and a warm query is imperceptible.

### P3 — Packaging + in-app surface

Bundle `ido-mcp` as a **Tauri sidecar** (`bundle.externalBin`, per-target-triple naming:
`binaries/ido-mcp-aarch64-apple-darwin`, `…-x86_64-pc-windows-msvc.exe`, …). Settings pane: enable
the server, copy the `.mcp.json` snippet, show/trigger the model download, show index status. Reuse
the same index for **in-app semantic search** in the `Ctrl+K` palette — same code, second consumer,
and the honest test of whether retrieval is actually good.

### P4 — Writes (gated) + polish

`create_entry`, `append_to_entry`, `create_task`, `update_task_field`. Off unless `--allow-write`.
Every write is undoable by construction (the store already models soft-delete + restore). Then the
optional extras: reranking, prompts, int8 quantization, `list_wells` + a `well` parameter.

---

## 8. Write tools — the rules, when we get there

Deliberately deferred, but the constraints are worth writing down now so P1–P3 don't foreclose
them:

- **Opt-in at the process level.** `--allow-write`; absent ⇒ the write tools are not advertised
  at all (not advertised-and-erroring — an unadvertised tool can't be attempted).
- **Never destructive.** No `delete_*`, no overwrite of an existing body. `append_to_entry` and
  `create_entry` only; edits to tasks are field-level. The undo path already exists in the store.
- **Path confinement** on every id: reject `..`, absolute paths, and anything resolving outside the
  well (`paths::valid_name` already rejects separators — extend to canonicalized-prefix checks).
- **Confirmation** is the client's job, not ours. Under 2026-07-28, server-initiated elicitation is
  gone; the MRTR pattern (`resultType: "input_required"`) is the path if we ever need it. Don't.
- **Slug collisions uniquify** rather than erroring (same rule as `create_task`).

---

## 9. Security and privacy

The server reads a folder of the user's private thoughts and hands them to a model. Take it
seriously:

| Concern | Position |
|---|---|
| Network | **None.** No outbound requests at runtime. The only network event in the system's life is the one-time model download, which is user-triggered and reported |
| Auth | None needed — stdio, spawned as the user, no listening socket. This is why stdio-before-remote matters |
| Path traversal | Canonicalize every id against the well root; reject escapes. Never follow symlinks out |
| Secrets in the well | A well may contain credentials the user pasted into a note. We can't classify that. **Document it**: enabling the MCP server exposes the whole well to whatever client is configured |
| Prompt injection | **Well content is data, not instructions.** A note containing "ignore previous instructions and…" is returned as tool output. Wrap returned bodies in a clear content delimiter and say so in the tool description; the client's model is the last line of defence, but the framing helps |
| Logging | stderr only (stdout is JSON-RPC). Never log entry bodies at default level |
| Assets | `read_asset` is deliberately **not** exposed as a tool in P1. Binary blobs in tool results are a bad deal for context; add later only with an explicit, sized reason |

---

## 10. Open questions — resolve with spikes, not opinions

1. **ORT packaging.** Does `ort` 2.0.0-rc.13 bundle cleanly into a Tauri app on macOS + Windows +
   Linux, or does it need a per-platform dylib dance? *Spike: build a 40-line binary that embeds one
   string with `BGESmallENV15`, bundle it as a sidecar, run it on all three.* **This gates P2/P3.**
2. **rmcp 3.0 churn.** How much does `3.0.0-beta.2` → `3.0.0` move? *Spike: build the seven tools
   against the beta, keep the tool bodies in `ido-store` so the SDK layer stays thin and swappable.*
3. **Chunk size.** 450 tokens with 15% overlap is a starting guess. The eval set decides.
4. **Where in-app semantic search lands** — does the palette become hybrid, or does semantic get a
   separate mode? Product question; P3.
5. **Model default** — is EmbeddingGemma-300M-Q4 good enough to skip BGE entirely and go
   multilingual from day one? Measure both on the eval set.
6. **Session/registry access from the MCP process** — `wells.json` lives in Tauri's
   `app_data_dir`, which the store crate can't ask Tauri for. Resolve the path with `directories`
   or replicate the platform rules; only needed for the `--well` fallback.

---

## Appendix A — client wiring

Project-scoped, in `.mcp.json`:

```jsonc
{
  "mcpServers": {
    "ido": {
      "command": "ido-mcp",
      "args": ["--well", "/Users/me/well"],
      "transport": "stdio"
    }
  }
}
```

Or user-scoped, once, so every project sees it:

```sh
claude mcp add --scope user ido -- ido-mcp --well ~/well
```

Shipped-app path (P3): the sidecar lives inside the bundle, and the settings pane offers a
copy-pastable snippet pointing at it, since the binary is not on `PATH`.

## Appendix B — commands this adds

```sh
just mcp                    # run ido-mcp against the most recent well, stderr to terminal
just mcp-inspect            # run under the MCP inspector for protocol-level debugging
cargo test -p ido-store     # the real store test suite (moved from -p ido)
cargo run -p ido-mcp -- --well <path> --reindex   # force a full index rebuild
cargo run -p ido-mcp -- --eval  docs/eval.jsonl   # retrieval eval: recall@5 / MRR
```

`just verify` gains `cargo test -p ido-store` and `cargo test -p ido-mcp`; clippy already runs
`--workspace`.

## Appendix C — references

- [`vendor/latent-design/docs/knowledge-architecture.md`](../vendor/latent-design/docs/knowledge-architecture.md) — the ecosystem plan this implements
- MCP spec 2026-07-28 — [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog), [blog](https://blog.modelcontextprotocol.io/posts/2026-07-28/)
- [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) — official Rust MCP SDK
- [fastembed-rs](https://github.com/anush008/fastembed-rs) · [docs.rs](https://docs.rs/fastembed)
- [EmbeddingGemma model card](https://ai.google.dev/gemma/docs/embeddinggemma/model_card) · [BGE-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5)
- [Writing effective tools for AI agents](https://www.anthropic.com/engineering/writing-tools-for-agents) — Anthropic
- [Reciprocal Rank Fusion explained](https://blog.serghei.pl/posts/reciprocal-rank-fusion-explained/) · [Hybrid search](https://weaviate.io/blog/hybrid-search-explained)
- [Tauri sidecars](https://v2.tauri.app/develop/sidecar/)
