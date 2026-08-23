//! End-to-end: spawn the built `ido-mcp` binary against a throwaway well and
//! talk MCP to it over stdio, exactly the way a client does.
//!
//! The well is seeded through `ido-store` (the same code the server reads back
//! with), so the fixture can never drift from the on-disk format. The client is
//! rmcp's own — `().serve(TokioChildProcess::new(…))` — so the handshake, the
//! schemas, and the framing are all exercised for real rather than mocked.

use chrono::{Duration, Local};
use rmcp::RoleClient;
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, GetPromptRequestParams, GetPromptResult,
    ReadResourceRequestParams, ReadResourceResult, ResourceContents,
};
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

/// The four write tools `--allow-write` adds, after the read ones (§8).
const WRITE_TOOLS: [&str; 4] = [
    "create_entry",
    "append_to_entry",
    "create_task",
    "update_task_field",
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

/// A second, smaller well for the prompt tests, whose dates are computed
/// against the real clock rather than pinned to a fixed year — `daily_review`
/// and `weekly_digest` both compare against `chrono::Local::now()`, so a
/// fixture with hard-coded 2026 dates would only be overdue/due-today by
/// accident of when the test happens to run.
///
/// One task overdue (due yesterday), one due today, one whose due date is
/// yesterday but which is already in the done column (proving a completed
/// task never counts as overdue), one freshly-written note, and one goal with
/// a task completed today (so it shows as "moved" within the last 7 days).
fn seed_well_for_prompts() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let well = dir.path().to_string_lossy().into_owned();
    wells::migrate_well(well.clone()).expect("scaffold");

    let today = Local::now().format("%Y-%m-%d").to_string();
    let yesterday = (Local::now() - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();

    tasks::create_task(
        well.clone(),
        "todo".into(),
        "Renew the certificate".into(),
        Some(yesterday.clone()),
    )
    .expect("overdue task");

    tasks::create_task(
        well.clone(),
        "todo".into(),
        "Ship the newsletter".into(),
        Some(today),
    )
    .expect("due-today task");

    let stale_but_done = tasks::create_task(
        well.clone(),
        "todo".into(),
        "Old already-done thing".into(),
        Some(yesterday),
    )
    .expect("task");
    tasks::move_task(well.clone(), stale_but_done, "done".into()).expect("complete");

    notes::write_note(
        well.clone(),
        "journal/today".into(),
        "Freshly written note.\n".into(),
    )
    .expect("note");

    let goal = tasks::create_goal(well.clone()).expect("goal");
    let goal = tasks::rename_goal(well.clone(), goal, "Launch week".into()).expect("rename");
    let goal_task = tasks::create_task(
        well.clone(),
        "todo".into(),
        "Finish the launch checklist".into(),
        None,
    )
    .expect("task");
    tasks::set_task_field(well.clone(), goal_task.clone(), "goal".into(), goal).expect("link");
    tasks::move_task(well.clone(), goal_task, "done".into()).expect("complete");

    dir
}

/// Spawn the built binary against `well` and complete the MCP handshake — the
/// default, read-only way a client gets it.
async fn connect(well: &TempDir) -> RunningService<RoleClient, ()> {
    spawn(well, false).await
}

/// Spawn the built binary, optionally with `--allow-write`.
async fn spawn(well: &TempDir, allow_write: bool) -> RunningService<RoleClient, ()> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ido-mcp"));
    command.arg("--well").arg(well.path());
    if allow_write {
        command.arg("--allow-write");
    }
    let transport = TokioChildProcess::new(command).expect("spawn ido-mcp");
    ().serve(transport).await.expect("mcp handshake")
}

/// The tool names the connected server advertises, in order.
async fn tool_names(client: &RunningService<RoleClient, ()>) -> Vec<String> {
    client
        .list_all_tools()
        .await
        .expect("tools/list")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// Read one entry's rendered body back — the round-trip every write response
/// promises.
async fn read_back(client: &RunningService<RoleClient, ()>, kind: &str, id: &str) -> String {
    ok_text(client, "get_entry", json!({ "kind": kind, "id": id })).await
}

/// The id a write response reports, taken from its `- id: \`…\`` line — parsed
/// out of the rendered markdown on purpose, so the assertion proves that what a
/// model *reads* is what the next call accepts.
fn reported_id(rendered: &str) -> String {
    let line = rendered
        .lines()
        .find(|l| l.starts_with("- id: "))
        .unwrap_or_else(|| panic!("no reported id in:\n{rendered}"));
    line.split('`').nth(1).expect("quoted id").to_string()
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

/// The text content of a `resources/read` result, concatenated — mirrors
/// [`text`] for tool results, since a resource body is never binary here.
fn resource_text(result: &ReadResourceResult) -> String {
    result
        .contents
        .iter()
        .filter_map(|c| match c {
            ResourceContents::TextResourceContents { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text content of a `prompts/get` result's messages, concatenated.
fn prompt_text(result: &GetPromptResult) -> String {
    result
        .messages
        .iter()
        .filter_map(|m| match &m.content {
            ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
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
    let names = tool_names(&client).await;
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

/// §8's gate, from the client's side: without `--allow-write` a write tool is
/// not in `tools/list` at all, and calling its name anyway is refused by the
/// router — "an unadvertised tool can't be attempted", enforced twice.
#[tokio::test]
async fn without_the_flag_the_write_tools_are_neither_listed_nor_callable() {
    let well = seed_well();
    let client = connect(&well).await;

    let names = tool_names(&client).await;
    for tool in WRITE_TOOLS {
        assert!(
            !names.contains(&tool.to_string()),
            "`{tool}` must not be advertised by a read-only server: {names:?}"
        );
    }

    // Nothing in the protocol stops a client calling a name it never saw.
    let params = CallToolRequestParams::new("create_entry").with_arguments(
        json!({ "kind": "note", "id": "smuggled", "content": "x" })
            .as_object()
            .expect("object args")
            .clone(),
    );
    let refusal = client.call_tool(params).await;
    let message = match refusal {
        Ok(result) => format!("served it: {}", text(&result)),
        Err(e) => e.to_string(),
    };
    assert!(
        message.contains("not found"),
        "an unadvertised write tool must be refused, but the server {message}"
    );

    // And the well is untouched — no half-written note from the attempt.
    assert!(
        notes::read_note(
            well.path().to_string_lossy().into_owned(),
            "smuggled".into()
        )
        .is_err(),
        "the refused call must not have created anything"
    );

    // The read-only server also says so in the one place a model looks first.
    let info = ok_text(&client, "well_info", json!({})).await;
    assert!(info.contains("- access: read-only"), "{info}");

    client.cancel().await.expect("shutdown");
}

/// With the flag: `create_entry` really writes, and a second create at the same
/// id uniquifies instead of clobbering (§8 — never destructive).
#[tokio::test]
async fn allow_write_creates_entries_without_ever_clobbering() {
    let well = seed_well();
    let path = well.path().to_string_lossy().into_owned();
    let client = spawn(&well, true).await;

    let names = tool_names(&client).await;
    assert_eq!(
        names[..EXPECTED_TOOLS.len()],
        EXPECTED_TOOLS,
        "the read tools keep their order and come first"
    );
    assert_eq!(
        names[EXPECTED_TOOLS.len()..],
        WRITE_TOOLS,
        "orientation first, mutation last"
    );
    let info = ok_text(&client, "well_info", json!({})).await;
    assert!(info.contains("- access: write-enabled"), "{info}");

    // A fresh id lands exactly where it was asked for.
    let created = ok_text(
        &client,
        "create_entry",
        json!({
            "kind": "note",
            "id": "projects/latency",
            "content": "# Latency\n\nBudget: 100ms.\n",
        }),
    )
    .await;
    assert_eq!(reported_id(&created), "projects/latency", "{created}");
    assert_eq!(
        notes::read_note(path.clone(), "projects/latency".into()).expect("the file exists"),
        "# Latency\n\nBudget: 100ms.\n",
        "create_entry writes the body verbatim"
    );
    // The reported id round-trips into a read.
    let read = read_back(&client, "note", "projects/latency").await;
    assert!(read.contains("Budget: 100ms."), "{read}");

    // The same id again: uniquified, and the first note is untouched.
    let again = ok_text(
        &client,
        "create_entry",
        json!({
            "kind": "note",
            "id": "projects/latency",
            "content": "a different note\n",
        }),
    )
    .await;
    let second = reported_id(&again);
    assert_ne!(second, "projects/latency", "a taken id must uniquify");
    assert!(
        again.contains("already taken") && again.contains("nothing was overwritten"),
        "the response must say the id it used is not the one asked for:\n{again}"
    );
    assert_eq!(
        notes::read_note(path.clone(), "projects/latency".into()).unwrap(),
        "# Latency\n\nBudget: 100ms.\n",
        "the original note was clobbered"
    );
    assert_eq!(
        notes::read_note(path.clone(), second.clone()).unwrap(),
        "a different note\n"
    );

    // Wiki pages follow the same rule against the seeded `auth-system`.
    let page = ok_text(
        &client,
        "create_entry",
        json!({ "kind": "wiki", "id": "auth-system", "content": "impostor\n" }),
    )
    .await;
    assert_ne!(reported_id(&page), "auth-system", "a taken slug uniquifies");
    assert!(
        wiki::read_page(path.clone(), "auth-system".into()).contains("How tokens are issued"),
        "the original page was clobbered"
    );

    // Tasks are somebody else's job, and the refusal says whose.
    let wrong_tool = call(
        &client,
        "create_entry",
        json!({ "kind": "task", "id": "x", "content": "y" }),
    )
    .await;
    assert_eq!(wrong_tool.is_error, Some(true));
    assert!(
        text(&wrong_tool).contains("create_task"),
        "{}",
        text(&wrong_tool)
    );

    // Path confinement still applies to every write id.
    for id in ["../escape", "/etc/passwd"] {
        let escape = call(
            &client,
            "create_entry",
            json!({ "kind": "note", "id": id, "content": "x" }),
        )
        .await;
        assert_eq!(escape.is_error, Some(true), "`{id}` must be refused");
        assert!(text(&escape).contains("invalid id"), "{}", text(&escape));
    }

    client.cancel().await.expect("shutdown");
}

/// `resources/templates/list` (§5.3): the four `ido://<kind>/{id}` templates,
/// each with a real description and a markdown mime type.
#[tokio::test]
async fn lists_the_four_resource_templates() {
    let well = seed_well();
    let client = connect(&well).await;

    let templates = client
        .list_all_resource_templates()
        .await
        .expect("resources/templates/list");
    let uris: Vec<&str> = templates.iter().map(|t| t.uri_template.as_str()).collect();
    for expected in [
        "ido://note/{path}",
        "ido://wiki/{slug}",
        "ido://task/{id}",
        "ido://goal/{id}",
    ] {
        assert!(uris.contains(&expected), "missing `{expected}`: {uris:?}");
    }
    for template in &templates {
        assert!(
            template.description.as_deref().unwrap_or_default().len() > 20,
            "`{}` needs a real description",
            template.name
        );
        assert_eq!(template.mime_type.as_deref(), Some("text/markdown"));
    }

    client.cancel().await.expect("shutdown");
}

/// `resources/list` (§5.3) is bounded, and one of its entries reads back
/// through `resources/read` as the same delimited body `get_entry` returns.
#[tokio::test]
async fn resources_list_is_bounded_and_a_listed_uri_reads_back() {
    let well = seed_well();
    let client = connect(&well).await;

    let listed = client.list_all_resources().await.expect("resources/list");
    assert!(!listed.is_empty(), "the seeded well has entries to list");
    assert!(
        listed.len() <= 50,
        "resources/list must stay bounded, got {}",
        listed.len()
    );
    for resource in &listed {
        assert!(resource.uri.starts_with("ido://"), "{}", resource.uri);
        assert_eq!(resource.mime_type.as_deref(), Some("text/markdown"));
    }

    let note = listed
        .iter()
        .find(|r| r.uri == "ido://note/projects/auth-notes")
        .expect("the seeded note is in the listing");
    let read = client
        .read_resource(ReadResourceRequestParams::new(note.uri.clone()))
        .await
        .expect("resources/read");
    let body = resource_text(&read);
    assert!(body.contains("BEGIN WELL CONTENT"), "{body}");
    assert!(body.contains(TOKEN), "{body}");

    client.cancel().await.expect("shutdown");
}

/// `resources/read`'s id round-trips from a `search` hit exactly the way
/// `get_entry`'s does — same kind, same id, same delimited body — because
/// both paths call the same renderer (§5.3).
#[tokio::test]
async fn resources_read_round_trips_a_search_hit_id() {
    let well = seed_well();
    let client = connect(&well).await;

    let hits = ok_text(&client, "search", json!({ "query": TOKEN })).await;
    let (kind, id) = first_hit(&hits);
    let uri = format!("ido://{kind}/{id}");

    let via_resource = resource_text(
        &client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .expect("resources/read"),
    );
    let via_tool = read_back(&client, &kind, &id).await;
    assert!(via_resource.contains(TOKEN), "{via_resource}");
    assert_eq!(
        via_resource, via_tool,
        "a resource read and a get_entry call of the same id must return identical text"
    );

    client.cancel().await.expect("shutdown");
}

/// A malformed uri, an unknown kind, a traversal id, and a well-formed uri
/// naming an entry that doesn't exist are all refused — and the server keeps
/// serving afterwards (§9).
#[tokio::test]
async fn resources_read_refuses_bad_uris_without_dying() {
    let well = seed_well();
    let client = connect(&well).await;

    for uri in [
        "file:///etc/passwd",
        "ido://recipe/x",
        "ido://note/../../etc/passwd",
        "ido://wiki/does-not-exist-here",
    ] {
        let result = client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await;
        assert!(result.is_err(), "`{uri}` must be refused, not served");
    }

    let still_alive = ok_text(&client, "well_info", json!({})).await;
    assert!(
        still_alive.contains("# well:"),
        "the server survives a run of bad resource reads"
    );

    client.cancel().await.expect("shutdown");
}

/// `prompts/list` (§5.4): both prompts, described, taking no arguments.
#[tokio::test]
async fn both_prompts_are_listed_with_descriptions_and_no_arguments() {
    let well = seed_well();
    let client = connect(&well).await;

    let prompts = client.list_all_prompts().await.expect("prompts/list");
    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"daily_review"), "{names:?}");
    assert!(names.contains(&"weekly_digest"), "{names:?}");
    for prompt in &prompts {
        assert!(
            prompt.description.as_deref().unwrap_or_default().len() > 20,
            "`{}` needs a real description",
            prompt.name
        );
        assert!(
            prompt.arguments.is_none(),
            "`{}` shouldn't need arguments — it reads the whole well",
            prompt.name
        );
    }

    client.cancel().await.expect("shutdown");
}

/// `daily_review` (§5.4) surfaces an overdue task and a due-today one, leaves
/// out a task whose due date is in the past but which is already done, and
/// mentions the freshly-written note.
#[tokio::test]
async fn daily_review_triages_overdue_due_today_and_recent_notes() {
    let well = seed_well_for_prompts();
    let client = connect(&well).await;

    let result = client
        .get_prompt(GetPromptRequestParams::new("daily_review"))
        .await
        .expect("prompts/get daily_review");
    let message = prompt_text(&result);

    assert!(message.contains("renew-the-certificate"), "{message}");
    assert!(message.contains("ship-the-newsletter"), "{message}");
    assert!(
        !message.contains("Old already-done thing"),
        "a completed task must never read as overdue, however old its due date:\n{message}"
    );
    assert!(
        message.contains("journal/today") || message.contains("Freshly written note"),
        "{message}"
    );

    client.cancel().await.expect("shutdown");
}

/// `weekly_digest` (§5.4) reports the freshly-written note as changed and the
/// goal that gained a completed task in the last 7 days.
#[tokio::test]
async fn weekly_digest_reports_changes_and_goal_progress() {
    let well = seed_well_for_prompts();
    let client = connect(&well).await;

    let result = client
        .get_prompt(GetPromptRequestParams::new("weekly_digest"))
        .await
        .expect("prompts/get weekly_digest");
    let message = prompt_text(&result);

    assert!(
        message.contains("journal/today") || message.contains("Freshly written note"),
        "the note written moments ago should show as changed:\n{message}"
    );
    assert!(message.contains("Launch week"), "{message}");
    assert!(
        message.contains("1/1"),
        "one task linked to the goal, and it's done:\n{message}"
    );

    client.cancel().await.expect("shutdown");
}

/// There is no third prompt, so an unknown name is refused rather than
/// silently answering with one of the other two.
#[tokio::test]
async fn get_prompt_refuses_an_unknown_name() {
    let well = seed_well();
    let client = connect(&well).await;

    let result = client
        .get_prompt(GetPromptRequestParams::new("monthly_digest"))
        .await;
    assert!(result.is_err(), "an unknown prompt name must be refused");

    client.cancel().await.expect("shutdown");
}

/// `append_to_entry` grows a body and leaves everything else alone — including,
/// for a task, its whole frontmatter.
#[tokio::test]
async fn appending_grows_a_body_and_leaves_frontmatter_intact() {
    let well = seed_well();
    let path = well.path().to_string_lossy().into_owned();
    let client = spawn(&well, true).await;

    let before = tasks::list_tasks(path.clone())
        .into_iter()
        .find(|t| t.id == "write-the-migration")
        .expect("seeded task");

    let appended = ok_text(
        &client,
        "append_to_entry",
        json!({
            "kind": "task",
            "id": "write-the-migration",
            "text": "- [ ] deploy behind a flag",
        }),
    )
    .await;
    assert_eq!(reported_id(&appended), "write-the-migration");
    assert!(appended.contains("frontmatter: untouched"), "{appended}");

    let after = tasks::list_tasks(path.clone())
        .into_iter()
        .find(|t| t.id == "write-the-migration")
        .expect("still there");
    assert!(
        after.body.contains("Depends on [[auth-system]]"),
        "what was written before must survive an append:\n{}",
        after.body
    );
    assert!(
        after.body.contains("- [ ] deploy behind a flag"),
        "{}",
        after.body
    );
    assert!(
        after.body.starts_with(&before.body.trim_end().to_string()),
        "the append lands at the end, not the start:\n{}",
        after.body
    );
    assert_eq!(
        (
            after.status.as_str(),
            after.title.as_str(),
            after.completed.as_str(),
            after.archived
        ),
        (
            before.status.as_str(),
            before.title.as_str(),
            before.completed.as_str(),
            before.archived
        ),
        "no frontmatter field may move on an append"
    );
    assert_eq!(
        (after.checks_done, after.checks_total),
        (before.checks_done, before.checks_total + 1),
        "the store re-derived the checklist rollup from the grown body"
    );

    // A missing note is created by an append; a missing task is not.
    let fresh = ok_text(
        &client,
        "append_to_entry",
        json!({ "kind": "note", "id": "journal/2026-08-23", "text": "first line" }),
    )
    .await;
    assert!(fresh.contains("this call created it"), "{fresh}");
    assert_eq!(
        notes::read_note(path.clone(), "journal/2026-08-23".into())
            .unwrap()
            .trim(),
        "first line"
    );
    let phantom = call(
        &client,
        "append_to_entry",
        json!({ "kind": "task", "id": "no-such-task", "text": "x" }),
    )
    .await;
    assert_eq!(
        phantom.is_error,
        Some(true),
        "a typo must not invent a task"
    );
    assert!(text(&phantom).contains("list_tasks"), "{}", text(&phantom));

    client.cancel().await.expect("shutdown");
}

/// The task-shaped writes go through the board's own rules: a new card gets its
/// fields validated, and completing one is a status change — which is what
/// stamps `completed:`.
#[tokio::test]
async fn task_writes_are_field_level_and_keep_the_boards_invariants() {
    let well = seed_well();
    let path = well.path().to_string_lossy().into_owned();
    let client = spawn(&well, true).await;

    let created = ok_text(
        &client,
        "create_task",
        json!({
            "title": "Ship the write tools",
            "status": "in-progress",
            "due": "2026-09-15 09:30",
            "priority": "high",
            "tags": "mcp, p4",
            "goal": "q3-launch",
            "body": "- [ ] gate it\n",
        }),
    )
    .await;
    let id = reported_id(&created);
    assert_eq!(id, "ship-the-write-tools", "the id is the title's slug");
    let task = |id: &str| {
        tasks::list_tasks(path.clone())
            .into_iter()
            .find(|t| t.id == id)
            .expect("task on the board")
    };
    let fresh = task(&id);
    assert_eq!(fresh.status, "in-progress");
    assert_eq!(fresh.due, "2026-09-15 09:30", "the time of day is kept");
    assert_eq!(fresh.priority, "high");
    assert_eq!(fresh.tags, vec!["mcp".to_string(), "p4".into()]);
    assert_eq!(fresh.goal, "q3-launch");
    assert_eq!(fresh.checks_total, 1, "the body landed");
    assert!(fresh.completed.is_empty());

    // Validation happens against *this* well, before anything is written.
    for (args, expect) in [
        (
            json!({ "title": "x", "status": "shipping" }),
            "board columns",
        ),
        (
            json!({ "title": "x", "due": "next tuesday" }),
            "must be a date",
        ),
        (
            json!({ "title": "x", "priority": "urgent" }),
            "not a priority",
        ),
        (
            json!({ "title": "x", "goal": "no-such-goal" }),
            "list_goals",
        ),
        // Frontmatter is one `key: value` per line, written verbatim, so a
        // newline in a free-text field would smuggle in a second field —
        // including one the allow-list refuses by name.
        (
            json!({ "title": "x", "tags": "urgent\ncompleted: 2026-01-01" }),
            "single line",
        ),
    ] {
        let refused = call(&client, "create_task", args.clone()).await;
        assert_eq!(refused.is_error, Some(true), "{args} must be refused");
        assert!(
            text(&refused).contains(expect),
            "the refusal should name the problem: {}",
            text(&refused)
        );
    }
    assert_eq!(
        tasks::list_tasks(path.clone())
            .iter()
            .filter(|t| t.title == "x")
            .count(),
        0,
        "a refused create must leave no card behind"
    );

    // Completing a task is a status change, and the store stamps the date.
    let done = ok_text(
        &client,
        "update_task_field",
        json!({ "id": id, "field": "status", "value": "done" }),
    )
    .await;
    assert!(done.contains("- was: in-progress"), "{done}");
    assert!(done.contains("stamped `completed:`"), "{done}");
    let completed = task(&id);
    assert_eq!(completed.status, "done");
    assert!(
        !completed.completed.is_empty(),
        "moving into the done column must stamp the completion date"
    );
    assert_eq!(
        completed.checks_total, 1,
        "the body is untouched by a field edit"
    );

    // Idempotent: the same write again changes nothing.
    let again = ok_text(
        &client,
        "update_task_field",
        json!({ "id": id, "field": "status", "value": "done" }),
    )
    .await;
    assert!(
        !again.contains("stamped"),
        "a no-op must not re-stamp:\n{again}"
    );
    assert_eq!(task(&id).completed, completed.completed);

    // The bookkeeping fields are refused *by name*, with the reason.
    for (field, expect) in [("completed", "done column"), ("archived", "auto-archive")] {
        let refused = call(
            &client,
            "update_task_field",
            json!({ "id": id, "field": field, "value": "2026-01-01" }),
        )
        .await;
        assert_eq!(refused.is_error, Some(true), "`{field}` must be refused");
        let message = text(&refused);
        assert!(message.contains(expect), "`{field}` gave: {message}");
        assert!(
            message.contains("title, status, priority, due, tags, goal, repeat"),
            "a refusal lists what is allowed: {message}"
        );
    }
    assert!(
        !task(&id).archived,
        "the refused write must not have landed"
    );

    // A rename re-slugs the file, and the response hands back the new id.
    let renamed = ok_text(
        &client,
        "update_task_field",
        json!({ "id": id, "field": "title", "value": "Ship P4" }),
    )
    .await;
    let new_id = reported_id(&renamed);
    assert_eq!(new_id, "ship-p4", "{renamed}");
    assert!(renamed.contains("the task's id is now"), "{renamed}");
    let read = read_back(&client, "task", &new_id).await;
    assert!(read.contains("Ship P4"), "the new id round-trips:\n{read}");

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
