//! End-to-end: spawn the built `ido-mcp` binary against a throwaway well and
//! talk MCP to it over stdio, exactly the way a client does.
//!
//! The well is seeded through `ido-store` (the same code the server reads back
//! with), so the fixture can never drift from the on-disk format. The client is
//! rmcp's own — `().serve(TokioChildProcess::new(…))` — so the handshake, the
//! schemas, and the framing are all exercised for real rather than mocked.

use rmcp::RoleClient;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use serde_json::json;
use tempfile::TempDir;

use ido_store::{notes, tasks, wells, wiki};

/// The tool names `tools/list` must report, in this order (see `server::TOOL_ORDER`).
const EXPECTED_TOOLS: [&str; 7] = [
    "well_info",
    "search",
    "get_entry",
    "list_entries",
    "backlinks",
    "list_tasks",
    "list_goals",
];

/// A distinctive token that appears in exactly one note body and one task
/// title, so a search for it has an unambiguous expected answer.
const TOKEN: &str = "zarquon";

/// Build a small but complete well: two notes (one linking to the wiki page and
/// mentioning [`TOKEN`]), one wiki page, four tasks across three columns (one
/// due-dated, one archived, one linked to a goal and completed), and one goal.
fn seed_well() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let well = dir.path().to_string_lossy().into_owned();
    wells::migrate_well(well.clone()).expect("scaffold");

    notes::write_note(
        well.clone(),
        "projects/auth-notes".into(),
        format!(
            "# Auth notes\n\nThe {TOKEN} protocol is ours. See [[auth-system]] for the design.\n"
        ),
    )
    .expect("note");
    notes::write_note(well.clone(), "inbox".into(), "loose thoughts\n".into()).expect("note");

    wiki::write_page(
        well.clone(),
        "auth-system".into(),
        "# Auth system\n\nHow tokens are issued and rotated.\n".into(),
    )
    .expect("page");

    let rotate = tasks::create_task(
        well.clone(),
        "todo".into(),
        format!("Rotate the {TOKEN} keys"),
        Some("2026-09-01".into()),
    )
    .expect("task");
    assert_eq!(
        rotate, "rotate-the-zarquon-keys",
        "slug is the title's slug"
    );

    let migration = tasks::create_task(
        well.clone(),
        "in-progress".into(),
        "Write the migration".into(),
        None,
    )
    .expect("task");
    tasks::update_task_body(
        well.clone(),
        migration,
        "Depends on [[auth-system]].\n\n- [x] draft\n- [ ] review\n".into(),
    )
    .expect("body");

    let old =
        tasks::create_task(well.clone(), "todo".into(), "Old idea".into(), None).expect("task");
    tasks::set_task_field(well.clone(), old, "archived".into(), "true".into()).expect("archive");

    // A goal with one linked task, moved into the done column so progress is 1/1
    // and `completed:` gets stamped by the store's own status path.
    let goal = tasks::create_goal(well.clone()).expect("goal");
    let goal = tasks::rename_goal(well.clone(), goal, "Q3 launch".into()).expect("rename");
    tasks::set_goal_field(
        well.clone(),
        goal.clone(),
        "target".into(),
        "2026-09-30".into(),
    )
    .expect("target");
    let ship = tasks::create_task(
        well.clone(),
        "todo".into(),
        "Ship the prototype".into(),
        None,
    )
    .expect("task");
    tasks::set_task_field(well.clone(), ship.clone(), "goal".into(), goal).expect("link");
    tasks::move_task(well, ship, "done".into()).expect("complete");

    dir
}

/// Spawn the built binary against `well` and complete the MCP handshake.
async fn connect(well: &TempDir) -> RunningService<RoleClient, ()> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ido-mcp"));
    command.arg("--well").arg(well.path());
    let transport = TokioChildProcess::new(command).expect("spawn ido-mcp");
    ().serve(transport).await.expect("mcp handshake")
}

/// Call one tool and return its raw result (a tool-level error is a normal
/// result with `is_error`, not a transport failure).
async fn call(
    client: &RunningService<RoleClient, ()>,
    name: &'static str,
    args: serde_json::Value,
) -> CallToolResult {
    let params = CallToolRequestParams::new(name)
        .with_arguments(args.as_object().expect("object args").clone());
    client.call_tool(params).await.expect("tool call")
}

/// The text content of a tool result, concatenated.
fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Call a tool that is expected to succeed and return its markdown.
async fn ok_text(
    client: &RunningService<RoleClient, ()>,
    name: &'static str,
    args: serde_json::Value,
) -> String {
    let result = call(client, name, args).await;
    let body = text(&result);
    assert_ne!(
        result.is_error,
        Some(true),
        "`{name}` unexpectedly errored: {body}"
    );
    body
}

/// The ``kind`` and ``id`` of the first hit in `search`'s markdown — parsed out
/// of the rendered text on purpose, so the assertion proves that what a model
/// *reads* is what `get_entry` accepts.
fn first_hit(rendered: &str) -> (String, String) {
    let line = rendered
        .lines()
        .find(|l| l.starts_with("1. "))
        .unwrap_or_else(|| panic!("no ranked hit in:\n{rendered}"));
    let kind = line.split("**").nth(1).expect("bold kind").to_string();
    let id = line.split('`').nth(1).expect("quoted id").to_string();
    (kind, id)
}

#[tokio::test]
async fn serves_the_seven_read_only_tools_over_stdio() {
    let well = seed_well();
    let client = connect(&well).await;

    // --- tools/list: exactly seven, in the documented order -----------------
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .expect("tools/list")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, EXPECTED_TOOLS, "fixed, orientation-first ordering");

    // --- well_info: the shape of the well, and which search mode is live ----
    let info = ok_text(&client, "well_info", json!({})).await;
    assert!(
        info.contains("- semantic index: not built"),
        "well_info must report the index state:\n{info}"
    );
    assert!(
        info.contains("- search: keyword-only"),
        "a build with no index answers keyword-only, and says so:\n{info}"
    );
    assert!(info.contains("notes: 2 files"), "{info}");
    assert!(info.contains("wiki: 1 pages"), "{info}");
    assert!(info.contains("tasks: 3 active, 1 archived"), "{info}");
    assert!(info.contains("goals: 1 active, 0 archived"), "{info}");
    assert!(
        info.contains("`todo` → `planning` → `in-progress` → `done`"),
        "{info}"
    );

    // --- search → get_entry: the id round-trips verbatim --------------------
    let hits = ok_text(&client, "search", json!({ "query": TOKEN })).await;
    assert!(hits.contains("projects/auth-notes"), "{hits}");
    assert!(hits.contains("rotate-the-zarquon-keys"), "{hits}");
    let (kind, id) = first_hit(&hits);
    let entry = ok_text(&client, "get_entry", json!({ "kind": kind, "id": id })).await;
    assert!(
        entry.contains("BEGIN WELL CONTENT"),
        "bodies are delimited as data:\n{entry}"
    );
    assert!(
        entry.contains(TOKEN),
        "the round-tripped id must fetch the matching body:\n{entry}"
    );

    // Section scoping filters the same hits down.
    let wiki_only = ok_text(
        &client,
        "search",
        json!({ "query": "auth", "section": "wiki" }),
    )
    .await;
    assert!(wiki_only.contains("`auth-system`"), "{wiki_only}");
    assert!(!wiki_only.contains("**note**"), "{wiki_only}");
    let bad_section = call(
        &client,
        "search",
        json!({ "query": "a", "section": "notez" }),
    )
    .await;
    assert_eq!(
        bad_section.is_error,
        Some(true),
        "unknown section is rejected"
    );

    // --- search modes: every mode answers, and never overstates itself ------
    for mode in ["hybrid", "semantic", "keyword"] {
        let rendered = ok_text(&client, "search", json!({ "query": TOKEN, "mode": mode })).await;
        assert!(
            rendered.contains("rotate-the-zarquon-keys"),
            "`{mode}` must still find the literal token:\n{rendered}"
        );
        // This build has no index, so hybrid/semantic degrade — and must say
        // so rather than let an empty answer read as "the well has nothing".
        if mode == "keyword" {
            assert!(
                rendered.contains("(keyword search)"),
                "the heading names the mode that ran:\n{rendered}"
            );
            assert!(
                !rendered.contains("semantic index unavailable"),
                "keyword never asked for the index:\n{rendered}"
            );
        } else {
            assert!(
                rendered.contains("semantic index unavailable:"),
                "`{mode}` degraded silently:\n{rendered}"
            );
            assert!(
                rendered.contains("keyword results"),
                "a degraded response says what it *is*:\n{rendered}"
            );
            assert!(
                rendered.contains("(keyword search)"),
                "and the heading names the mode that actually ran:\n{rendered}"
            );
        }
    }
    let bad_mode = call(
        &client,
        "search",
        json!({ "query": "a", "mode": "psychic" }),
    )
    .await;
    assert_eq!(
        bad_mode.is_error,
        Some(true),
        "an unknown mode is an error, not a silent default"
    );

    // --- list_entries: the cheap map, with round-tripping ids ---------------
    let listed = ok_text(&client, "list_entries", json!({ "section": "notes" })).await;
    assert!(listed.contains("`projects/auth-notes`"), "{listed}");
    assert!(listed.contains("`inbox`"), "{listed}");
    let scoped = ok_text(
        &client,
        "list_entries",
        json!({ "section": "notes", "prefix": "projects" }),
    )
    .await;
    assert!(scoped.contains("`projects/auth-notes`"), "{scoped}");
    assert!(
        !scoped.contains("`inbox`"),
        "prefix scopes to the folder:\n{scoped}"
    );
    let task_map = ok_text(&client, "list_entries", json!({ "section": "tasks" })).await;
    assert!(
        task_map.contains("| goal | `q3-launch`"),
        "goals are listed too:\n{task_map}"
    );

    // --- list_tasks: filters, and archived excluded by default --------------
    let todo = ok_text(&client, "list_tasks", json!({ "status": "todo" })).await;
    assert!(todo.contains("rotate-the-zarquon-keys"), "{todo}");
    assert!(
        !todo.contains("old-idea"),
        "archived tasks are excluded unless asked for:\n{todo}"
    );
    let with_archived = ok_text(
        &client,
        "list_tasks",
        json!({ "status": "todo", "include_archived": true }),
    )
    .await;
    assert!(with_archived.contains("old-idea"), "{with_archived}");
    assert!(with_archived.contains("(archived)"), "{with_archived}");
    let dated = ok_text(&client, "list_tasks", json!({ "due_after": "2026-08-01" })).await;
    assert!(dated.contains("2026-09-01"), "{dated}");
    assert!(
        !dated.contains("write-the-migration"),
        "undated excluded:\n{dated}"
    );
    let bad_date = call(&client, "list_tasks", json!({ "due_after": "soon" })).await;
    assert_eq!(bad_date.is_error, Some(true), "a junk date is a tool error");

    // --- list_goals: progress derived from the linked tasks -----------------
    let goals = ok_text(&client, "list_goals", json!({})).await;
    assert!(goals.contains("`q3-launch`"), "{goals}");
    assert!(goals.contains("Q3 launch"), "{goals}");
    assert!(
        goals.contains("| 1/1 |"),
        "one linked task, in the done column:\n{goals}"
    );

    // --- backlinks: cross-section, notes *and* task bodies ------------------
    let links = ok_text(&client, "backlinks", json!({ "slug": "auth-system" })).await;
    assert!(
        links.contains("projects/auth-notes"),
        "the note links here:\n{links}"
    );
    assert!(
        links.contains("write-the-migration"),
        "the task body links here:\n{links}"
    );

    // --- path confinement: an escape is an error result, not a crash --------
    for (tool, args) in [
        ("get_entry", json!({ "kind": "note", "id": "../x" })),
        ("get_entry", json!({ "kind": "note", "id": "/etc/passwd" })),
        ("backlinks", json!({ "slug": "../../secrets" })),
        (
            "list_entries",
            json!({ "section": "notes", "prefix": "../.." }),
        ),
    ] {
        let result = call(&client, tool, args.clone()).await;
        assert_eq!(
            result.is_error,
            Some(true),
            "`{tool}` must refuse {args}, not serve it"
        );
        assert!(
            text(&result).contains("invalid"),
            "the refusal should say why: {}",
            text(&result)
        );
    }

    // An unknown kind is likewise a tool error, and the server keeps serving.
    let unknown = call(&client, "get_entry", json!({ "kind": "recipe", "id": "x" })).await;
    assert_eq!(unknown.is_error, Some(true));
    let still_alive = ok_text(&client, "well_info", json!({})).await;
    assert!(
        still_alive.contains("# well:"),
        "the server survives bad input"
    );

    client.cancel().await.expect("shutdown");
}

/// The well must be exactly as it was found: a read-only server that scaffolds,
/// migrates or sweeps on open would silently rewrite a user's folder.
///
/// This is the **default-feature** build, which has no embedder at all — so the
/// bar is absolute: not one byte, anywhere, including `.ido/`.
#[cfg(not(feature = "semantic"))]
#[tokio::test]
async fn serving_a_well_writes_nothing() {
    let well = seed_well();
    let before = snapshot(well.path());
    exercise(&well).await;
    assert_eq!(before, snapshot(well.path()), "the well was modified");
}

/// With semantic search compiled in, §6.6 sanctions exactly one write: the
/// rebuildable index cache under `.ido/index/`. **Well content is still
/// untouchable.**
///
/// On a machine without the model this passes trivially (nothing is written at
/// all, because nothing can embed) — which is itself the contract worth
/// pinning: an absent model must never turn into a half-built index or a
/// scaffolded well.
#[cfg(feature = "semantic")]
#[tokio::test]
async fn serving_a_well_writes_only_the_index_cache() {
    let well = seed_well();
    let before = snapshot(well.path());
    exercise(&well).await;
    // The startup sweep is detached; give it a moment to land before looking.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let after = snapshot(well.path());

    let changed: Vec<&(String, u64)> = after.iter().filter(|e| !before.contains(e)).collect();
    for (path, _) in &changed {
        assert!(
            path.starts_with(".ido/index/"),
            "only the rebuildable index cache may be written, but `{path}` changed"
        );
    }
    let content_before: Vec<_> = before
        .iter()
        .filter(|(p, _)| !p.starts_with(".ido/index/"))
        .collect();
    let content_after: Vec<_> = after
        .iter()
        .filter(|(p, _)| !p.starts_with(".ido/index/"))
        .collect();
    assert_eq!(content_before, content_after, "well content was modified");
    eprintln!(
        "index files written: {}",
        if changed.is_empty() {
            "none (no model on this machine — the degradation path)".to_string()
        } else {
            changed
                .iter()
                .map(|(p, _)| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
}

/// Drive one session's worth of reads across every tool that touches disk.
async fn exercise(well: &TempDir) {
    let client = connect(well).await;
    let _ = ok_text(&client, "well_info", json!({})).await;
    let _ = ok_text(&client, "search", json!({ "query": TOKEN })).await;
    let _ = ok_text(
        &client,
        "search",
        json!({ "query": TOKEN, "mode": "semantic" }),
    )
    .await;
    let _ = ok_text(&client, "list_tasks", json!({})).await;
    let _ = ok_text(
        &client,
        "get_entry",
        json!({ "kind": "wiki", "id": "auth-system" }),
    )
    .await;
    client.cancel().await.expect("shutdown");
}

/// Every file under `root`, as sorted `(relative path, length)` pairs.
fn snapshot(root: &std::path::Path) -> Vec<(String, u64)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if let Ok(meta) = entry.metadata() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}
