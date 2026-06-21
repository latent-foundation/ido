use latent_ui::theme::{initial_theme, setup_theme_effect};
use latent_ui::ThemeToggle;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

// Tauri IPC. Commands live in `src-tauri/src/lib.rs`; this is the only bridge.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

#[derive(Clone, Serialize, Deserialize)]
struct NoteMeta {
    id: String,
    title: String,
}

#[derive(Serialize)]
struct IdArgs {
    id: String,
}

#[derive(Serialize)]
struct WriteArgs {
    id: String,
    content: String,
}

/// Mirror of the backend's title rule, so the sidebar updates live while typing.
fn derive_title(content: &str, id: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| id.to_string())
}

#[component]
pub fn App() -> impl IntoView {
    let is_dark = RwSignal::new(initial_theme());
    provide_context(is_dark);
    setup_theme_effect(is_dark);

    let notes = RwSignal::new(Vec::<NoteMeta>::new());
    let active = RwSignal::new(None::<String>);
    let editor = NodeRef::<leptos::html::Textarea>::new();

    // Open a note: fetch its markdown and push it into the (uncontrolled) editor.
    let open_note = move |id: String| {
        active.set(Some(id.clone()));
        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&IdArgs { id }).unwrap();
            if let Ok(js) = invoke("read_note", args).await {
                if let Some(ta) = editor.get_untracked() {
                    ta.set_value(&js.as_string().unwrap_or_default());
                }
            }
        });
    };

    // Initial load: list notes, open the first one if there is any.
    spawn_local(async move {
        if let Ok(js) = invoke("list_notes", JsValue::null()).await {
            let list: Vec<NoteMeta> = serde_wasm_bindgen::from_value(js).unwrap_or_default();
            if let Some(first) = list.first() {
                open_note(first.id.clone());
            }
            notes.set(list);
        }
    });

    let new_note = move |_| {
        spawn_local(async move {
            if let Ok(js) = invoke("create_note", JsValue::null()).await {
                if let Ok(meta) = serde_wasm_bindgen::from_value::<NoteMeta>(js) {
                    let id = meta.id.clone();
                    notes.update(|list| list.push(meta));
                    open_note(id);
                }
            }
        });
    };

    // Autosave on every keystroke; also refresh the sidebar title live.
    let on_edit = move |ev| {
        let Some(id) = active.get_untracked() else {
            return;
        };
        let content = event_target_value(&ev);
        let title = derive_title(&content, &id);
        notes.update(|list| {
            if let Some(n) = list.iter_mut().find(|n| n.id == id) {
                n.title = title;
            }
        });
        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&WriteArgs { id, content }).unwrap();
            let _ = invoke("write_note", args).await;
        });
    };

    view! {
        <div class="ido-app">
            <aside class="ido-sidebar">
                <div class="ido-brand">
                    <span class="ido-wordmark">"ido"</span>
                    <span class="ido-kanji">"井戸"</span>
                </div>

                <div class="ido-actions">
                    <button class="ido-new" on:click=new_note>
                        "+ New note"
                    </button>
                </div>

                <nav class="ido-list">
                    {move || {
                        let current = active.get();
                        notes
                            .get()
                            .into_iter()
                            .map(|n| {
                                let id = n.id.clone();
                                let selected = current.as_deref() == Some(id.as_str());
                                view! {
                                    <button
                                        class="ido-list-item"
                                        class:active=selected
                                        on:click=move |_| open_note(id.clone())
                                    >
                                        {n.title}
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                </nav>

                <div class="ido-status">
                    <span class="ido-status-left">
                        <span class="ido-dot"></span>
                        "local"
                    </span>
                    <ThemeToggle />
                </div>
            </aside>

            <main class="ido-main">
                <textarea
                    class="ido-editor"
                    node_ref=editor
                    on:input=on_edit
                    prop:disabled=move || active.get().is_none()
                    placeholder="Select a note, or create one to begin writing…"
                ></textarea>
            </main>
        </div>
    }
}
