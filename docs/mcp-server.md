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
→ embed locally with [candle](https://github.com/huggingface/candle) (**pure Rust, no ONNX
Runtime, nothing native to bundle**) → **brute-force cosine over an in-memory f32 matrix** (no ANN
index — a personal well is far too small to need one) → fuse with the existing lexical scan via
reciprocal rank fusion. Writes come last, gated, and only after the read path is trusted.

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
- **A local LLM in-app.** The embedding model here is a ~130 MB retrieval model, not a generative
  one. The Gemma/on-device-assistant ambition in [`CLAUDE.md`](../CLAUDE.md) is adjacent and now
  shares more than plumbing — candle runs generative models too, so the model-cache and device
  setup built here is directly reusable — but it is out of scope.
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

**[`rmcp`](https://crates.io/crates/rmcp) 3.1.0** implements the stable 2026-07-28 spec with
back-compat to 2025-11-25 and earlier. 3.0 has shipped final — the beta this doc originally
targeted is obsolete. Features needed:

```toml
rmcp = { version = "3.1", features = ["server", "macros", "transport-io", "schemars"] }
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

Keep the tool *bodies* in `ido-store` and the `rmcp` layer thin regardless — the SDK is young and
the spec moves; a thin seam makes an SDK bump a one-file change.

**As-built corrections (2026-08-21)**

Verified during implementation against rmcp 3.1.4:
- `serve()` comes from `rmcp::ServiceExt` and takes the handler value (not a Result): `Ido::open(&well).serve(stdio()).await?.waiting().await?`.
- `Parameters` lives at `rmcp::handler::server::wrapper::Parameters` (not re-exported at the crate root).
- `#[tool_router(server_handler)]` works but forwards no `instructions`; the two-macro form is needed for a real instructions block: `#[tool_router]` on the inherent impl + `#[tool_handler(router = self.tool_router)]` on a hand-written `impl ServerHandler` with `get_info`.
- `ToolRouter::list_all()` sorts tools alphabetically — fixed tool order requires hand-implementing `list_tools` with a `TOOL_ORDER` const.
- Tool errors: returning `Result<String, String>` maps `Err` to a `CallToolResult` with `isError: true`.
- `rmcp` re-exports `schemars` (`use rmcp::schemars`).
- `#[tool(annotations(read_only_hint = true, open_world_hint = false))]` is available and used on every tool.
- Measured dep cost: rmcp added 11 crates to this workspace (not 3 as originally estimated).

See `crates/ido-mcp/src/server.rs` for the reference implementation.

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

### 6.3 Embedding model — Candle, not ONNX Runtime

**[`candle`](https://github.com/huggingface/candle) 0.11** (`candle-core` + `candle-nn` +
`candle-transformers`), HuggingFace's minimalist Rust ML framework. The decisive property is
narrower than "pure Rust" — state it precisely, because the imprecise version is false:

> **Candle ships no native runtime artifact.** Everything it needs is compiled *into* the binary
> at build time. There is no `.dylib`/`.dll`/`.so` to bundle beside the executable, to locate at
> load time, or to sign and notarize separately.

That, not "no C code", is what kills the largest packaging risk in this plan. `fastembed` is a
nicer *API*, but it sits on `ort` 2.0.0-rc.x, whose ONNX Runtime is a **shared library that must
travel with the app** and be found at load time on three platforms inside a Tauri bundle. Candle
trades a lower level of abstraction for a bundle that is one file.

**Candle does still compile some C**, and the dependency audit in §11 documents exactly what and
why the `default-features = false` trick does *not* avoid it. Short version: `candle-core` itself
depends on `tokenizers` with `features = ["onig"]`, so Oniguruma (C) is compiled in whatever we
declare. It needs a C compiler on the *build* machine — which every Tauri build already requires —
and produces no artifact to ship.

**Dependencies:**

```toml
candle-core         = "0.11"
candle-nn           = "0.11"
candle-transformers = "0.11"
tokenizers = { version = "0.22", default-features = false }  # see §11 — does not do what you think
ureq       = { version = "3",    features = ["rustls"] }     # model download only
safetensors = "0.8"   # transitively via candle; listed for clarity
```

Optional acceleration, all opt-in cargo features, none required:

| Feature | Platform | Effect |
|---|---|---|
| `metal` | macOS | GPU via Metal. System framework — **nothing to bundle**. Big win on index builds |
| `accelerate` | macOS | BLAS via the Accelerate system framework. Also nothing to bundle |
| `mkl` | x86 Linux/Windows | Intel MKL. **Pulls a large static blob** — probably not worth it |
| `cuda` | NVIDIA | Irrelevant here; a note-taking app should not require CUDA |

Default to plain CPU everywhere. Treat `metal` and `accelerate` as measured optimizations for the
macOS build, not requirements (§10).

#### Model candidates

Candle needs the model as **safetensors + `config.json` + `tokenizer.json`**, and it needs an
architecture that `candle-transformers` implements. Confirmed present in 0.11: `bert`,
`distilbert`, `jina_bert`, `nomic_bert`, `modernbert`, `xlm_roberta`, `stella_en_v5`.

| Model | Arch | Dim | Disk (f32) | Pooling | Notes |
|---|---|---|---|---|---|
| **`BAAI/bge-small-en-v1.5`** | `bert` | 384 | ~133 MB | **CLS** | **Recommended start.** Best retrieval quality per byte, English-first, tiny vectors |
| `sentence-transformers/all-MiniLM-L6-v2` | `bert` | 384 | ~90 MB | mean | Smallest, slightly weaker. Fallback for constrained machines; it's also candle's own example model, so it's the best-trodden path for the spike |
| `intfloat/multilingual-e5-small` | `xlm_roberta` | 384 | ~470 MB | mean | The multilingual option. Disk cost is vocabulary, not depth |
| `nomic-ai/modernbert-embed-base` | `modernbert` | 768 | ~600 MB | mean | Long context (8k), strongest of these — but 4× the download and 2× the vector width |

Ship `bge-small-en-v1.5` as the default; put the choice in `.ido/well.toml` (or app settings). The
index manifest records **which model built it**, so switching triggers a rebuild rather than
silently mixing vector spaces.

#### EmbeddingGemma — deferred, not rejected

It's the model we'd *want*: 768-dim with Matryoshka truncation to 512/256/128, 100+ languages, an
8k context, explicitly designed for on-device use, and the obvious bridge to the Gemma ambition in
[`CLAUDE.md`](../CLAUDE.md). Four things stand between us and it, and they're worth writing down
precisely rather than hand-waving:

| Blocker | Detail | Severity |
|---|---|---|
| **Not in candle-transformers** | 0.11 has Gemma 3 as a *causal* architecture. EmbeddingGemma is the same backbone with **bidirectional** attention, mean pooling, and two dense projection heads — an encoder, not a decoder. Needs a port. **But HF's own [`text-embeddings-inference`](https://github.com/huggingface/text-embeddings-inference) already has a candle Gemma3 encoder** (Apache-2.0, handles the sliding/full attention pattern), so it's adaptation, not invention | Medium |
| **1.21 GB, fp32 only** | `model.safetensors` is **1,211,486,072 bytes**, stored F32 — and TEI's candle backend rejects `--dtype float16` outright ("Gemma3 is only supported in fp32 precision"). Add a **33 MB `tokenizer.json`** for the 262k-token vocabulary. That's ~9× bge-small on disk and ~1.3 GB resident in a process the MCP client spawns per session | **High** |
| **Gated repo** | `google/embeddinggemma-300m` is `gated: "manual"` — it requires a HuggingFace account and manual acceptance of Google's license. A plain `GET .../resolve/main/model.safetensors` **401s**. Our whole download design (§6.3) assumes an anonymous fetch. The outs are all bad: make the user paste an HF token, mirror the weights ourselves (needs a license read), or trust a community re-upload | **High** |
| **Weights are split across four files** | The pooling and both dense heads live outside the main checkpoint in sentence-transformers module layout — `1_Pooling/config.json`, `2_Dense/` and `3_Dense/` (9.4 MB each). Candle would load three safetensors files and apply the projections by hand | Low |

**The quantized escape hatch doesn't currently work either.** Google publishes QAT checkpoints and
the community publishes GGUF conversions at ~200 MB, and llama.cpp has learned to carry the
sentence-transformers dense modules inside the GGUF. But candle's quantized support is built around
its *generative* model list, so a Gemma3 **encoder** + two dense heads from GGUF is a second,
different port — and there are field reports of at least one QAT GGUF producing embeddings that
don't match the reference implementation at all. Two ports and a correctness cloud is not a
starting position.

**Verdict: P4 upgrade with named triggers, not the P2 default.** Revisit when *either* of these
becomes true:

1. `candle-transformers` ships a Gemma3 encoder upstream (drops blocker 1 and probably 2), **or**
2. we decide multilingual retrieval is a requirement rather than a nice-to-have — at which point
   compare against `multilingual-e5-small` first, since it is 470 MB, ungated, and runs on the
   `xlm_roberta` architecture candle **already** supports.

The `ModelSpec` registry above is what makes this cheap to revisit: EmbeddingGemma's asymmetric
prompts (`"task: search result | query: {q}"` for queries, `"title: none | text: {doc}"` for
documents) are just two more fields, and the manifest's model id already forces a clean reindex on
a switch. Design for the swap; don't pay for it now.

#### Alternatives considered — is anything more Rust-native?

Candle is chosen, but it is not the only pure-ish-Rust option and it is **not the most Rust-native
one** — worth recording honestly, so the trade is a decision rather than an oversight. All figures
below are **measured** the same way as §11 (resolve the manifest, diff crate names against ido's
lockfile):

| Stack | Net-new crates | Third-party C in the tree | Model format | Verdict |
|---|---|---|---|---|
| **candle** 0.11 | 82 | **oniguruma** (forced) + **ring** | safetensors, arch must exist in `candle-transformers` | **Chosen** |
| **[`tract`](https://github.com/sonos/tract)** 0.23 | 81 | **none** — `cc` builds tract's *own* asm kernels | ONNX export | Documented escape hatch |
| **[`model2vec-rs`](https://github.com/MinishLab/model2vec-rs)** 0.2 | 74 | oniguruma + ring | safetensors static table | Fallback / de-risking play |
| **[`burn`](https://github.com/tracel-ai/burn)** 0.19 | **372** | ring | ONNX → Rust codegen | **Rejected** |
| ~~`ort`/fastembed~~ | — | ONNX Runtime **shared library** | ONNX | Rejected — §6.3 opening |

**tract is the genuinely more Rust-native answer.** Sonos's inference engine, in production on
millions of devices, and the only candidate here that pulls **no third-party C library at all**:
no oniguruma, no ring. It does use `cc`, but for `tract-linalg` compiling *its own* hand-written
SIMD/assembly kernels — which is why it's fast, and is self-contained rather than a vendored
third-party dependency. It also doesn't force `tokenizers/onig`, so unlike under candle our
`default-features = false` actually works.

Its cost is a **format hop and a coverage question**: models must be ONNX (fine —
`Xenova/bge-small-en-v1.5` publishes one, and even potion ships `onnx/model.onnx`), and tract
passes ~85% of the ONNX backend test suite, so any given graph loading is not guaranteed.

**Decision: candle, and this is settled rather than pending a spike.** The reasons are not about
C-purity, which tract wins:

- **Native safetensors, no export step.** Every model swap under tract needs someone to produce and
  trust an ONNX conversion; under candle it's three files from the model's own repo.
- **The architecture zoo is the product.** `bert`, `xlm_roberta`, `modernbert`, `nomic_bert`,
  `jina_bert`, `stella_en_v5` are all already there, which is what makes the `ModelSpec` registry
  a config change rather than a project.
- **It tracks HF's models because it is HF's framework**, and the EmbeddingGemma path runs through
  it.
- **A missing model is a tractable problem** rather than someone else's ONNX-export problem —
  relevant only if we ever hit one, which `bge-small-en-v1.5` means we don't.

Oniguruma and ring are compiled statically and ship nothing (§11.2), which makes the purity gap
real but small. Tract stays documented here as the **escape hatch**, not a pending decision: if
candle's C surface, compile time or op coverage ever becomes a genuine problem, the `Embedder`
trait below means swapping it is one file and a re-index, and this section is the note-to-self
explaining why it's viable.

**model2vec is a different trade entirely — worth understanding, because it's tempting.** It
replaces the transformer forward pass with a *static lookup table*: distilled per-token vectors,
pooled. No attention, no context. That makes indexing effectively free. The quality cost is real
and measurable (MTEB **retrieval** sub-score, which is the only column that matters here):

| Model | MTEB retrieval | Note |
|---|---|---|
| `bge-small-en-v1.5` (candle default) | ~highest of these | Our baseline |
| `all-MiniLM-L6-v2` | **42.92** | The *weaker* candle candidate |
| `potion-retrieval-32M` | **35.06** | Best static retrieval model — **~82% of MiniLM** |
| `potion-base-8M` | 31.11 | |

`potion-retrieval-32M` is MIT, **ungated**, 129 MB at f32 (i8 weights are supported, ~4× smaller),
and a 1.5 MB tokenizer. Note the f32 size is *no smaller than bge-small* — the win is compute and
RAM, not disk.

So: **not the default** — semantic search is the headline feature and a 20% retrieval haircut is
the wrong thing to economize on. But it is an excellent **P2 de-risking vehicle**: build the whole
pipeline (chunker, `vectors.bin`, RRF, eval harness, `well_info` reporting) against a trivial
embedder that indexes in seconds, prove the plumbing, then swap the embedder. Which leads to the
one design decision this section actually forces:

> **Put the embedder behind a trait.** `trait Embedder { fn dim(&self) -> usize; fn embed(&self,
> texts: &[String], role: Role) -> Result<Vec<Vec<f32>>>; }` with `Role::{Query, Document}` so the
> `ModelSpec` prefixes and pooling live behind it. Candle, tract and model2vec then differ in one
> file, the eval harness can score all three on the same well, and the manifest's model id already
> forces a clean reindex on a switch.

Burn is out: 372 net-new crates for a feature this size is disqualifying, whatever its merits as a
training framework.

#### The model registry — what fastembed was doing for us

This is the part that is easy to get wrong and produces *plausible but quietly bad* results, which
is the worst failure mode there is. fastembed hid it; with candle we own it. Encode it as data,
once:

```rust
pub struct ModelSpec {
    pub repo:         &'static str,  // "BAAI/bge-small-en-v1.5"
    pub arch:         Arch,          // Bert | XlmRoberta | ModernBert | NomicBert
    pub dim:          usize,         // 384
    pub max_tokens:   usize,         // 512
    pub pooling:      Pooling,       // Cls | Mean   ← per-model, NOT a global choice
    pub query_prefix: &'static str,  // BGE: "Represent this sentence for searching relevant passages: "
    pub doc_prefix:   &'static str,  // BGE: ""     E5: "passage: " (and query: "query: ")
    pub normalize:    bool,          // true for all four above
}
```

Two traps live in that struct:

1. **Pooling is per-model.** BGE pools the **CLS token**; MiniLM, E5 and Nomic take the
   **attention-masked mean**. Use mean pooling on BGE and you get a working, normalized,
   entirely-mediocre index — no error, just worse recall you'd never trace back.
2. **Asymmetric prefixes are load-bearing.** BGE-v1.5 expects the query (never the document) to
   carry `"Represent this sentence for searching relevant passages: "`. E5 expects `query: ` and
   `passage: ` on their respective sides. Omitting them costs real recall; applying the query
   prefix to documents costs more. The index build path and the query path must read this from the
   **same** `ModelSpec`.

Both belong in the eval harness (§6.7) as explicit regression cases.

#### The embedding path, concretely

Mirrors [candle's own BERT example](https://github.com/huggingface/candle/tree/main/candle-examples/examples/bert):

```rust
// load once, keep resident for the process lifetime
let cfg: Config       = serde_json::from_str(&fs::read_to_string(config_path)?)?;
let tokenizer         = Tokenizer::from_file(tok_path)?;      // + PaddingParams::BatchLongest
let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DTYPE, &device)? };
let model             = BertModel::load(vb, &cfg)?;

// per batch
let enc          = tokenizer.encode_batch(texts, true)?;      // texts already prefixed per ModelSpec
let token_ids    = Tensor::new(ids,   &device)?;              // [batch, seq]
let attn_mask    = Tensor::new(mask,  &device)?;
let token_types  = token_ids.zeros_like()?;
let hidden       = model.forward(&token_ids, &token_types, Some(&attn_mask))?;  // [batch, seq, dim]
let pooled       = match spec.pooling {
    Pooling::Cls  => hidden.i((.., 0))?,                      // first token
    Pooling::Mean => masked_mean(&hidden, &attn_mask)?,       // Σ(h·m) / Σm — never a naive .mean(1)
};
let embeddings   = normalize_l2(&pooled)?;
```

`DTYPE` is `F32`; candle's CPU path has no useful f16 story, and at 384 dims the memory is
irrelevant. `masked_mean` must divide by the **mask sum**, not the sequence length — padding
tokens otherwise drag every short chunk toward the same point in the space.

Batch 16–64 chunks per forward pass and reuse the loaded model across the whole index build; the
per-call overhead dominates otherwise.

#### Model download and cache

Candle's examples use `hf-hub`, but that crate went through a **0.5 → 1.0 API redesign** (candle
0.11 pins 0.5.0; 1.0.0 landed July 2026 with a different surface). We need exactly three files
from one public repo, so skip the dependency and fetch them directly:

```
https://huggingface.co/{repo}/resolve/main/{config.json,tokenizer.json,model.safetensors}
```

with `ureq` + rustls, into **`<app_data_dir>/models/{repo}/`** — *not* `.ido/`. Models are
per-machine and shared across wells; `.ido/` is per-well rebuildable cache. Verify a sha256 per
file against a pinned manifest, download to a temp path and rename into place, and treat a partial
download as absent. This is the **only** network event in the system's entire life (§9), it is
user-triggered, and it must be reported as such in the UI.

#### Honest costs

| Concern | Assessment |
|---|---|
| **Inference speed** | Candle CPU is generally **slower than ORT** for BERT — call it 1.5–3× until measured. It does not matter for queries (one short text, single-digit to low-tens ms) and it does matter for a cold index build. Measure it (§10) |
| **Cold index build** | The real exposure. 5,000 chunks × ~450 tokens on CPU is plausibly 1–5 min. Mitigations, in order: batch properly, `rayon` across batches, then the `metal`/`accelerate` features, then a smaller model. Build once in the background, incrementally thereafter (§6.6) — a user should meet this exactly once |
| **Compile time** | **Measured: 2m09s cold** for the 185-crate candle tree (§11.4). Gate the whole index module behind a **`semantic` cargo feature** on `ido-store` so `src-tauri`, `just verify`, and CI stay fast when it's off |
| **Binary size** | **Measured: 5.1 MB linked / 4.1 MB stripped** (§11.4). Model weights live in the on-disk cache, never in the executable |
| **Dependency count** | **+82 crates**, of which 79 are the candle stack and 3 are rmcp — see §11 |
| **Model correctness** | Candle reimplements architectures; a config key it doesn't parse can shift behavior. Pin the model revision and assert a **known-vector test**: embed one fixed string, compare to a committed reference vector within 1e-4 |

The `semantic` feature also preserves the old fallback: ship keyword-only, let semantic be an
opt-in build. `well_info` reports which mode is live either way.

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

**Reranking** — a cross-encoder over the fused top-20 — is a real quality lever but costs more
under candle than it would have under fastembed: `candle-transformers`' bert module exposes
`BertModel` and `BertForMaskedLM`, but no sequence-classification head, so a reranker means
loading the base encoder plus a hand-written pooled linear head and matching it to the checkpoint's
weight names. Plus a second model, second download, and per-query latency on 20 pairs. Phase 4,
measured against the eval set, dropped if it doesn't pay — and it probably doesn't at this scale.


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

### P0 — Extract `ido-store` *(no new features)* — **DONE 2026-08-21**

Move the pure modules out of `src-tauri`; `src-tauri` becomes command wrappers. Frontend untouched.
**Done when:** every existing backend test passes unmoved, `just verify` is green, and the app
behaves identically.

### P1 — Read-only MCP server, keyword search — **DONE 2026-08-21**

`crates/ido-mcp` with rmcp + stdio and the seven tools of §5.2, `search` backed by the existing
lexical scan. Registered in `.mcp.json`, driven from Claude Code against a real well.
**Done when:** an agent can answer "what's on my board and what did I write about X" without ido
running; tool results stay under budget on a large well; a `just mcp` recipe runs it. Resources
(§5.3) ship in P4; `search` omits the `mode` parameter until P2 adds semantic support.

### P2 — Semantic index

Chunker, the candle embedder (`ModelSpec` registry + pooling + prefixes), model download,
`vectors.bin` + manifest, brute-force cosine, RRF fusion, the eval harness. All behind the
`semantic` cargo feature. `mode` parameter on `search`; `well_info` reports index state. Graceful
degradation to keyword when the model is absent or the feature is off.
**Done when:** hybrid beats keyword on the eval set, the known-vector test pins the embedder, a
cold index builds in a few minutes on a 2,000-note well, and a warm query is imperceptible.

### P3 — Packaging + in-app surface

Bundle `ido-mcp` as a **Tauri sidecar** (`bundle.externalBin`, per-target-triple naming:
`binaries/ido-mcp-aarch64-apple-darwin`, `…-x86_64-pc-windows-msvc.exe`, …) — with candle this is
**one self-contained executable per triple** and no accompanying native libraries, which is the
whole reason this phase is small. Settings pane: enable
the server, copy the `.mcp.json` snippet, show/trigger the model download, show index status. Reuse
the same index for **in-app semantic search** in the `Ctrl+K` palette — same code, second consumer,
and the honest test of whether retrieval is actually good.

The non-semantic slice (sidecar bundling + the settings-pane config surface) shipped 2026-08-21 alongside P1; the semantic parts (model download UI, index status, in-app semantic search) remain.

### P4 — Writes (gated) + polish

`create_entry`, `append_to_entry`, `create_task`, `update_task_field`. Off unless `--allow-write`.
Every write is undoable by construction (the store already models soft-delete + restore). Then the
optional extras: reranking, prompts, int8 quantization, `list_wells` + a `well` parameter, and the
**EmbeddingGemma revisit** if either trigger in §6.3 has fired.

### Beyond the roadmap — low priority, after the above

Three of §6.3's blockers are gaps in candle itself rather than in ido: no
`BertForSequenceClassification` (the reranker head), no Gemma3 *encoder*, and no fp16/bf16 for it.
Each is upstreamable, and candle's bar for a model PR — port from the Python reference, show the
logits match, add an example — overlaps the known-vector test we build anyway. Noted only so the
option is on record. **It is not a phase, not a prerequisite, and not scheduled**: P2 ships on
`bge-small-en-v1.5` regardless, and nothing above waits on it.

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

1. **Does candle reproduce the reference embeddings?** Everything downstream is worthless if the
   vectors are subtly wrong. *Spike: a 60-line binary that loads `bge-small-en-v1.5`, embeds a
   fixed string with CLS pooling + L2 norm, and diffs against the vector `sentence-transformers`
   produces in Python. Within 1e-4 ⇒ commit it as the known-vector test.* **This gates P2** — and
   it is a much cheaper gate than the ORT-packaging spike it replaces.
   *(The tract variant of this spike is **not** scheduled — candle is decided, §6.3. Keep the
   known-vector test backend-agnostic anyway, so the escape hatch stays cheap to test.)*
2. ~~**rmcp 3.0 churn.**~~ **Resolved:** rmcp 3.0 shipped final and 3.1.0 is current; build against
   3.1. Keep the tool bodies in `ido-store` so the SDK layer stays thin and swappable.
3. **Chunk size.** 450 tokens with 15% overlap is a starting guess. The eval set decides.
4. **Where in-app semantic search lands** — does the palette become hybrid, or does semantic get a
   separate mode? Product question; P3.
5. **Index-build throughput, and whether Metal is worth it.** *Spike: time a 5,000-chunk build on
   the CPU build, then with `--features metal` and with `accelerate`, on the same Mac.* If CPU is
   already a couple of minutes, ship CPU-only and skip the per-platform feature matrix entirely.
   Candle's Metal backend has thinner op coverage than its CPU one — a fallback path must exist,
   and any Metal build has to pass the known-vector test of (1) too.
6. **Multilingual, and how much it costs.** Is `multilingual-e5-small` (~470 MB, mostly vocabulary)
   worth 3.5× the download over `bge-small-en-v1.5` for this user's actual wells? Measure both on
   the eval set. EmbeddingGemma would be the better answer to this question — recheck whether
   candle-transformers has shipped the encoder before settling.
7. **Session/registry access from the MCP process** — `wells.json` lives in Tauri's
   `app_data_dir`, which the store crate can't ask Tauri for. Resolve the path with `directories`
   or replicate the platform rules; only needed for the `--well` fallback.

---

## 11. Dependency audit — what this actually costs

Measured, not estimated: resolved with `cargo generate-lockfile` against real manifests and
diffed by crate name against ido's current `Cargo.lock` (**501 unique crates** today). Re-run the
numbers before committing; they will drift.

### 11.1 The headline

| | Net-new crates |
|---|---|
| `rmcp` 3.1 (`server`, `macros`, `transport-io`, `schemars`) | **11** (measured in this workspace, not 3) |
| candle stack (`candle-core` + `candle-nn` + `candle-transformers` + `tokenizers` + `ureq`) | **79** |
| **Total** | **82** → ~583 crates in the workspace |

**rmcp is nearly free, and that is the most important line in this table.** The original estimate
was three crates — `rmcp`, `rmcp-macros`, `tokio-macros` — because Tauri already drags in tokio,
serde, serde_json, schemars, futures, hyper, tracing and anyhow; as built it measured 11, the
extras being the schemars-1 derive + `darling` macro stack and the dev-only client/child-process
transport the integration test uses. Either number is a rounding error on a tree this size.

**All the weight is semantic search.** Which is exactly why the `semantic` cargo feature from §6.3
is load-bearing rather than cosmetic: with it off, `ido-mcp` costs 3 crates; with it on, 82.
Keep it off in `src-tauri`'s default build so app builds and `just verify` are untouched.

Where the 79 come from (subtrees overlap — `rayon` and `num_cpus` are shared):

| Subtree | ≈ crates | What it is |
|---|---|---|
| `tokenizers` | 34 | HF tokenizer: regex engines, unicode normalization, `derive_builder`, `monostate` |
| `gemm` | 31 | The SIMD linear-algebra kernel — `pulp`, `dyn-stack`, `raw-cpuid`, the `gemm-f32/f64/c32/c64/f16` family. This is candle's actual compute |
| `ureq` + rustls | 11 | Model download only — `rustls`, `ring`, `webpki-roots`, `untrusted` |
| candle itself | 3 | `candle-core`, `candle-nn`, `candle-transformers` |
| model IO | ~4 | `safetensors`, `memmap2`, `zip`, `zerocopy` |

### 11.2 Native code — correcting the "pure Rust" claim

An earlier draft of §6.3 said a default candle build is pure Rust and that
`tokenizers = { default-features = false }` keeps it that way. **Both halves are wrong**, and the
audit is how that surfaced:

```toml
# candle-core 0.11.0's own manifest — non-optional, non-dev:
[target.'cfg(not(target_arch = "wasm32"))'.dependencies.tokenizers]
version = "0.22.0"
features = ["onig"]          # ← Oniguruma, a C library
default-features = false
```

Cargo **unions** features across the graph, so candle-core enabling `onig` means we get
`onig_sys` and its bundled C regex engine no matter what our own manifest says. Declaring
`default-features = false` on our side is still correct hygiene — it keeps `progressbar` and
`esaxx_fast` off — but it cannot remove `onig`.

| Crate | Native content | Forced by | Ships an artifact? |
|---|---|---|---|
| `onig_sys` | Oniguruma, **C** | `candle-core` → `tokenizers/onig`. Unavoidable | No — static |
| `ring` | **C + assembly** | `rustls` ← `ureq`. Avoidable, see below | No — static |
| `esaxx-rs` | C++ **only** with its `cpp` feature | Present but `esaxx_fast` is off ⇒ **pure Rust path** | n/a |

**This does not weaken the case against ORT** — it sharpens what the case actually is. Every one
of these compiles *into* the executable. ORT's problem was categorically different: a shared
library that must be shipped beside the binary, found at load time, and signed/notarized on macOS.
A C compiler on the build machine is not a new requirement; every Tauri build already needs Xcode
CLT / MSVC / gcc.

If dropping `ring` is worth 11 crates, the lever is to move model download into the app (which
already has an HTTP stack) and have `ido-mcp` require pre-fetched weights. Only worth it if the
crate count starts mattering.

**The alternatives are not cheaper — they're differently shaped.** Same measurement method:

| Stack | Net-new crates | `cc` | oniguruma | `ring` |
|---|---|---|---|---|
| candle | 82 | yes | **yes** (forced by `candle-core`) | **yes** |
| tract | 81 | yes — *its own asm kernels* | **no** | **no** |
| model2vec-rs | 74 | yes | **yes** | **yes** |
| burn | 372 | yes | — | yes |

Crate *count* barely separates the top three; **what separates them is whose C it is.** Only tract
avoids third-party C entirely. See §6.3 for the quality and format trade-offs that go with it.

### 11.3 Version duplication

Mostly a non-event — ido's tree already carries several crates at two majors (`base64`,
`thiserror`, `syn`, `getrandom`). One genuinely new split:

- **`schemars`** — ido has 0.8.22 and 0.9.0 via Tauri; rmcp wants **1.2.2**. A third copy compiles.
  Compile-time cost only; the two never meet at a type boundary.

### 11.4 Measured build cost

Release profile, Apple Silicon, candle tree only:

| Measurement | Value |
|---|---|
| Cold build, 185 crates | **2 min 09 s** |
| Incremental rebuild of the leaf crate | **3.6 s** |
| Linked binary exercising `BertModel::load` + `Tokenizer::from_file` + `ureq` | **5.14 MB** (4.06 MB stripped) |

The binary number is the one worth internalizing: **~5 MB, not hundreds.** Candle's model zoo is
dead-code-eliminated down to the architectures actually referenced, and model weights live on disk
in the cache, never in the executable. The 2-minute cold build is a CI concern, not a developer
one — it is cached after the first run, and the `semantic` feature keeps it off the default path.

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

### Exist (P0 + P1 + P3 slice)
```sh
just mcp                    # run ido-mcp against the most recent well, stderr to terminal
just mcp-inspect            # run under the MCP inspector for protocol-level debugging
just sidecar                # build release ido-mcp and stage as Tauri sidecar binary
cargo test -p ido-store     # the real store test suite (81 tests, moved from -p ido)
cargo test -p ido-mcp       # unit + stdio integration tests (14 tests)
```

### Still future (P2, P4)
```sh
cargo run -p ido-mcp -- --well <path> --reindex   # force a full index rebuild (P2)
cargo run -p ido-mcp -- --eval  docs/eval.jsonl   # retrieval eval: recall@5 / MRR (P2)
```

`just verify` includes `cargo test -p ido-store`, `cargo test -p ido` (2), and `cargo test -p ido-mcp`; `just dev`/`dev-debug` depend on `sidecar`; `cargo tauri build` requires `just sidecar` run first.

## Appendix C — references

- [`vendor/latent-design/docs/knowledge-architecture.md`](../vendor/latent-design/docs/knowledge-architecture.md) — the ecosystem plan this implements
- MCP spec 2026-07-28 — [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog), [blog](https://blog.modelcontextprotocol.io/posts/2026-07-28/)
- [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) — official Rust MCP SDK
- [candle](https://github.com/huggingface/candle) — [BERT example](https://github.com/huggingface/candle/tree/main/candle-examples/examples/bert) (the reference implementation for §6.3) · [docs.rs](https://docs.rs/candle-transformers)
- Alternatives weighed in §6.3: [tract](https://github.com/sonos/tract) · [model2vec-rs](https://github.com/MinishLab/model2vec-rs) + [MTEB results](https://github.com/MinishLab/model2vec/blob/main/results/README.md) · [burn](https://github.com/tracel-ai/burn)
- [BGE-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5) · [all-MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2) · [multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small)
- [EmbeddingGemma model card](https://ai.google.dev/gemma/docs/embeddinggemma/model_card) — not yet an option under candle; recheck later
- [Writing effective tools for AI agents](https://www.anthropic.com/engineering/writing-tools-for-agents) — Anthropic
- [Reciprocal Rank Fusion explained](https://blog.serghei.pl/posts/reciprocal-rank-fusion-explained/) · [Hybrid search](https://weaviate.io/blog/hybrid-search-explained)
- [Tauri sidecars](https://v2.tauri.app/develop/sidecar/)
