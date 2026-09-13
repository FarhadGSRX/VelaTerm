use super::*;
use crate::host::{AppCtx, HeadlessHost};
use base64::Engine;
use std::sync::Arc;

struct Fixture {
    app: AppCtx,
    dir: PathBuf,
    notes: PathBuf,
    id: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("vkb-notebooks-{}", uuid::Uuid::new_v4()));
        let notes = dir.join("Notes");
        std::fs::create_dir_all(&notes).unwrap();
        let db = crate::db::Db::open(&dir.join("test.db")).unwrap();
        let app = AppCtx::Headless(Arc::new(HeadlessHost::new(dir.clone(), db)));
        let registered = dispatch(&app, "kb_register", &json!({"root":notes})).unwrap();
        Self {
            app,
            dir,
            notes,
            id: registered["id"].as_str().unwrap().into(),
        }
    }
    fn call(&self, cmd: &str, mut args: Value) -> Result<Value> {
        args["vaultId"] = json!(self.id);
        dispatch(&self.app, cmd, &args)
    }
    fn write(&self, path: &str, content: &str) {
        let p = self.notes.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    fn get(&self, path: &str) -> Value {
        self.call("kb_get", json!({"path":path})).unwrap()
    }
    fn import(&self, path: &str, content: &[u8]) -> Value {
        let plan = self
            .call(
                "kb_import_begin",
                json!({"destination":"","files":[{"path":path,"size":content.len()}]}),
            )
            .unwrap();
        self.call("kb_import_chunk",json!({"importId":plan["importId"],"index":0,"offset":0,"base64":base64::engine::general_purpose::STANDARD.encode(content)})).unwrap();
        self.call("kb_import_commit", json!({"importId":plan["importId"]}))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn existing_files_remain_the_source_of_truth_and_closing_keeps_them() {
    let f = Fixture::new();
    let body = "---\ntags: [旅行, planning]\n---\n# 我的笔记\n\nA [[Second]] link.\n";
    f.write("日常/First.md", body);
    f.write("Second.md", "# Second");
    f.write(".obsidian/app.json", "{\"test\":true}");
    let note = f.get("日常/First.md");
    assert_eq!(note["content"], body);
    assert_eq!(note["links"][0]["path"], "Second.md");
    assert!(note["tags"].as_array().unwrap().contains(&json!("旅行")));
    f.write("日常/First.md", "# Changed externally\n");
    assert_eq!(f.get("日常/First.md")["content"], "# Changed externally\n");
    f.call("kb_unregister", json!({})).unwrap();
    assert!(f.notes.join("日常/First.md").is_file());
    assert!(f.notes.join(".obsidian/app.json").is_file());
    assert!(
        dispatch(&f.app, "kb_overview", &json!({})).unwrap()["vaults"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stale_saves_preserve_external_edits_and_markdown_bytes() {
    let f = Fixture::new();
    f.write("Note.md", "---\naliases: [Original]\n---\ntext\n");
    let old = f.get("Note.md");
    let next = "---\naliases: [Original]\n---\ntext\n\nAdded\n";
    let saved = f
        .call(
            "kb_save",
            json!({"path":"Note.md","digest":old["digest"],"content":next}),
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.notes.join("Note.md")).unwrap(),
        next
    );
    f.write("Note.md", "external");
    assert_eq!(
        f.call(
            "kb_save",
            json!({"path":"Note.md","digest":saved["digest"],"content":"stale"})
        )
        .unwrap_err(),
        "kb_conflict"
    );
    assert_eq!(
        std::fs::read_to_string(f.notes.join("Note.md")).unwrap(),
        "external"
    );
}

#[test]
fn links_ignore_code_and_resolve_relative_paths_aliases_and_ambiguity() {
    let f = Fixture::new();
    f.write(
        "Topics/A.md",
        "[[B|label]] and [Target](../B.md#Heading)\n`[[fake]]`\n```md\n[[fake]]\n```\n",
    );
    f.write("B.md", "# Heading\n");
    let note = f.get("Topics/A.md");
    assert_eq!(note["links"].as_array().unwrap().len(), 2);
    assert_eq!(note["links"][0]["path"], "B.md");
    assert_eq!(f.get("B.md")["backlinks"][0]["path"], "Topics/A.md");
    f.write("A/Duplicate.md", "one");
    f.write("B/Duplicate.md", "two");
    f.write("Ambiguous.md", "[[Duplicate]]");
    assert!(f.get("Ambiguous.md")["links"][0]["path"].is_null());
    f.write("Imported/Topics/Book.md", "[[Topics/Other]]");
    f.write("Imported/Topics/Other.md", "# Other");
    assert_eq!(
        f.get("Imported/Topics/Book.md")["links"][0]["path"],
        "Imported/Topics/Other.md"
    );
}

#[test]
fn moving_folders_updates_incoming_and_relative_links_without_changing_code() {
    let f = Fixture::new();
    f.write("Notes/Topic.md", "[asset](../assets/pic.png)\n[[Other]]");
    f.write("Other.md", "[[Notes/Topic|主题]]\n`[[Notes/Topic]]`\n");
    f.write("assets/pic.png", "fixture");
    f.call(
        "kb_favorite",
        json!({"path":"Notes/Topic.md","favorite":true}),
    )
    .unwrap();
    let result = f
        .call("kb_move", json!({"from":"Notes","to":"Moved"}))
        .unwrap();
    assert_eq!(result["path"], "Moved");
    assert!(!f.notes.join("Notes").exists());
    assert!(f.notes.join("Moved/Topic.md").exists());
    assert_eq!(
        std::fs::read_to_string(f.notes.join("Other.md")).unwrap(),
        "[[Moved/Topic|主题]]\n`[[Notes/Topic]]`\n"
    );
    assert_eq!(f.get("Moved/Topic.md")["favorite"], true);
    assert_eq!(
        f.get("Moved/Topic.md")["links"][0]["path"],
        "assets/pic.png"
    );
}

#[test]
fn trash_restore_and_destination_collisions_keep_original_data() {
    let f = Fixture::new();
    f.write("Folder/Keep.md", "original");
    let deleted = f.call("kb_trash", json!({"path":"Folder"})).unwrap();
    assert!(!f.notes.join("Folder").exists());
    f.write("Folder/New.md", "new");
    assert_eq!(
        f.call("kb_restore", json!({"id":deleted["id"]}))
            .unwrap_err(),
        "kb_exists"
    );
    std::fs::remove_dir_all(f.notes.join("Folder")).unwrap();
    f.call("kb_restore", json!({"id":deleted["id"]})).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.notes.join("Folder/Keep.md")).unwrap(),
        "original"
    );
}

#[test]
fn folder_import_preserves_bytes_and_rejects_replacements_and_partial_uploads() {
    let f = Fixture::new();
    let content = b"---\ntags: [imported]\n---\n# Imported\n";
    let result = f.import("Imported/Note.md", content);
    assert_eq!(result["imported"], 1);
    assert_eq!(
        std::fs::read(f.notes.join("Imported/Note.md")).unwrap(),
        content
    );
    assert_eq!(
        f.call(
            "kb_import_begin",
            json!({"files":[{"path":"Imported/Note.md","size":0}]})
        )
        .unwrap_err(),
        "kb_exists"
    );
    let plan=f.call("kb_import_begin",json!({"files":[{"path":"partial.md","size":10},{"path":".obsidian/app.json","size":0}]})).unwrap();
    assert_eq!(plan["skipped"], 1);
    assert_eq!(
        f.call("kb_import_commit", json!({"importId":plan["importId"]}))
            .unwrap_err(),
        "kb_incomplete_import"
    );
    assert!(!f.notes.join("partial.md").exists());
    f.call("kb_import_abort", json!({"importId":plan["importId"]}))
        .unwrap();
    let binary = [0, 255, 128, 1, 9];
    f.import("Imported/attachment.bin", &binary);
    assert_eq!(
        std::fs::read(f.notes.join("Imported/attachment.bin")).unwrap(),
        binary
    );
}

#[test]
fn invalid_paths_and_symlinks_do_not_escape_the_vault() {
    let f = Fixture::new();
    f.write("Safe.md", "safe");
    for raw in [
        "../outside.md",
        "/tmp/outside.md",
        ".obsidian/app.json",
        "a/../../outside.md",
        "a\\..\\outside.md",
    ] {
        assert!(f.call("kb_get", json!({"path":raw})).is_err(), "{raw}");
        assert!(
            f.call("kb_import_begin", json!({"files":[{"path":raw,"size":0}]}))
                .is_err(),
            "{raw}"
        );
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&f.dir, f.notes.join("escape")).unwrap();
        assert_eq!(
            f.call(
                "kb_create",
                json!({"parent":"escape","name":"outside.md","kind":"note"})
            )
            .unwrap_err(),
            "kb_invalid_path"
        );
        assert_eq!(f.call("kb_tree", json!({})).unwrap()["skipped"], 1);
    }
    assert!(!f.dir.join("outside.md").exists());
}

#[test]
fn search_and_favorites_use_backend_state_and_existing_knowledge_survives() {
    let f = Fixture::new();
    f.write("Recipes/茶.md", "# 茶\n乌龙茶 brewing #生活\n");
    f.write("Work.md", "# 工作\n");
    f.call(
        "kb_favorite",
        json!({"path":"Recipes/茶.md","favorite":true}),
    )
    .unwrap();
    let favorites = f.call("kb_tree", json!({"filter":"favorites"})).unwrap();
    assert_eq!(favorites["entries"].as_array().unwrap().len(), 1);
    let results = dispatch(&f.app, "kb_search", &json!({"query":"茶 brewing"})).unwrap();
    assert_eq!(results["total"], 1);
    assert_eq!(results["entries"][0]["path"], "Recipes/茶.md");
    let old=crate::memory::dispatch(&f.app,"memory_save",&json!({"id":null,"version":0,"title":"Generated knowledge","summary":"","content":"saved independently","tags":[],"related":[]})).unwrap();
    f.call("kb_unregister", json!({})).unwrap();
    assert_eq!(
        crate::memory::dispatch(&f.app, "memory_get", &json!({"id":old["id"]})).unwrap()["entry"]
            ["content"],
        "saved independently"
    );
}

#[test]
fn search_reports_snippets_lines_scores_vault_scope_and_limit() {
    let f = Fixture::new();
    f.write(
        "Recipes/茶.md",
        "# 茶\nfirst line\n乌龙茶 brewing here #生活\n乌龙茶 again\n",
    );
    f.write("Notes/Pineapple.md", "# Draft\nnothing to see\n");
    f.write("Notes/body.md", "# Body\npineapple jam\n");
    f.write("Notes/one.md", "brimming one\n");
    f.write("Notes/two.md", "brimming two\n");
    f.write("Notes/three.md", "brimming three\n");
    f.call("kb_favorite", json!({"path":"Recipes/茶.md","favorite":true}))
        .unwrap();

    let hit = dispatch(&f.app, "kb_search", &json!({"query":"乌龙茶"})).unwrap();
    assert_eq!(hit["total"], 1);
    let entry = &hit["entries"][0];
    assert_eq!(entry["vaultId"], json!(f.id));
    assert_eq!(entry["path"], "Recipes/茶.md");
    assert_eq!(entry["name"], "茶.md");
    assert!(entry["absolutePath"]
        .as_str()
        .unwrap()
        .ends_with("Recipes/茶.md"));
    assert_eq!(entry["line"], 3);
    assert_eq!(entry["matches"], 2);
    assert!(entry["summary"].as_str().unwrap().contains("乌龙茶"));
    assert_eq!(entry["score"], 10);
    assert_eq!(entry["favorite"], true);
    assert_eq!(entry["tags"], json!(["生活"]));
    assert!(entry["updatedAt"].as_i64().unwrap() > 0);

    // A long matched line is clipped around the hit and marked on both trimmed ends.
    f.write(
        "Notes/long.md",
        &format!("{} needle {}", "x".repeat(300), "y".repeat(300)),
    );
    let clipped = dispatch(&f.app, "kb_search", &json!({"query":"needle"})).unwrap();
    assert_eq!(clipped["total"], 1);
    assert_eq!(clipped["entries"][0]["line"], 1);
    let summary = clipped["entries"][0]["summary"].as_str().unwrap();
    assert!(summary.contains("needle"));
    assert!(summary.starts_with('…') && summary.ends_with('…'));
    assert!(summary.chars().count() <= 200);

    // A name/path-only hit has no line and outranks a body-only hit.
    let ranked = dispatch(&f.app, "kb_search", &json!({"query":"pineapple"})).unwrap();
    assert_eq!(ranked["total"], 2);
    assert_eq!(ranked["entries"][0]["path"], "Notes/Pineapple.md");
    assert_eq!(ranked["entries"][0]["line"], 0);
    assert_eq!(ranked["entries"][0]["matches"], 0);
    assert_eq!(ranked["entries"][0]["summary"], "# Draft");
    assert_eq!(ranked["entries"][1]["path"], "Notes/body.md");
    assert_eq!(ranked["entries"][1]["line"], 2);
    assert!(
        ranked["entries"][0]["score"].as_u64().unwrap()
            > ranked["entries"][1]["score"].as_u64().unwrap()
    );

    let limited = dispatch(&f.app, "kb_search", &json!({"query":"brimming","limit":2})).unwrap();
    assert_eq!(limited["total"], 3);
    assert_eq!(limited["entries"].as_array().unwrap().len(), 2);
    assert_eq!(limited["hasMore"], true);
    let clamped = dispatch(&f.app, "kb_search", &json!({"query":"brimming","limit":0})).unwrap();
    assert_eq!(clamped["total"], 3);
    assert_eq!(clamped["entries"].as_array().unwrap().len(), 1);
    assert_eq!(clamped["hasMore"], true);
    let all = dispatch(&f.app, "kb_search", &json!({"query":"brimming"})).unwrap();
    assert_eq!(all["entries"].as_array().unwrap().len(), 3);
    assert_eq!(all["hasMore"], false);

    let empty = dispatch(&f.app, "kb_search", &json!({"query":"   "})).unwrap();
    assert_eq!(empty["total"], 0);
    assert!(empty["entries"].as_array().unwrap().is_empty());
    assert_eq!(
        dispatch(&f.app, "kb_search", &json!({"query":"a".repeat(1001)})).unwrap_err(),
        "kb_invalid"
    );

    let other = f.dir.join("Other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("Spice.md"), "saffron and brimming\n").unwrap();
    let other_id = dispatch(&f.app, "kb_register", &json!({"root":other})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let unscoped = dispatch(&f.app, "kb_search", &json!({"query":"saffron"})).unwrap();
    assert_eq!(unscoped["total"], 1);
    assert_eq!(unscoped["entries"][0]["vaultId"], json!(other_id));
    let scoped = dispatch(&f.app, "kb_search", &json!({"query":"saffron","vaultId":f.id})).unwrap();
    assert_eq!(scoped["total"], 0);
    assert!(scoped["entries"].as_array().unwrap().is_empty());
    assert!(scoped["unavailable"].as_array().unwrap().is_empty());
    let other_scoped =
        dispatch(&f.app, "kb_search", &json!({"query":"saffron","vaultId":other_id})).unwrap();
    assert_eq!(other_scoped["total"], 1);
    assert_eq!(other_scoped["entries"][0]["path"], "Spice.md");
}

#[test]
fn search_lists_directly_linked_notes_as_related() {
    let f = Fixture::new();
    f.write(
        "A.md",
        "# A\n限流设计记录\n- [[B]]\n- [[B]]\n- [[A]]\n- [see](C.md)\n",
    );
    f.write("B.md", "# B\nunrelated body\n");
    f.write("C.md", "# C\nunrelated body\n");
    f.write("D.md", "# D\n限流 other record [[B]]\n");
    let hits = dispatch(&f.app, "kb_search", &json!({"query":"限流"})).unwrap();
    assert_eq!(hits["total"], 2);
    let a = hits["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "A.md")
        .unwrap();
    let related = a["related"].as_array().unwrap();
    assert_eq!(related.len(), 2, "重复与自链接不重复出现: {related:?}");
    assert_eq!(related[0]["path"], "B.md");
    assert_eq!(related[0]["name"], "B.md");
    assert!(related[0]["absolutePath"].as_str().unwrap().ends_with("B.md"));
    assert_eq!(related[1]["path"], "C.md");
    let d = hits["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "D.md")
        .unwrap();
    assert_eq!(d["related"][0]["path"], "B.md");
    // Browsing the tree never returns the search-only field.
    let tree = f.call("kb_tree", json!({})).unwrap();
    assert!(tree["entries"][0].get("related").is_none());
}

#[test]
fn malformed_markdown_does_not_block_valid_notes_or_byte_preserving_imports() {
    let f = Fixture::new();
    f.write("Valid.md", "# Valid");
    let bytes = [0xff, 0xfe, 0, 1];
    f.import("Legacy.md", &bytes);
    let tree = f.call("kb_tree", json!({})).unwrap();
    assert_eq!(tree["entries"].as_array().unwrap().len(), 1);
    assert_eq!(tree["skipped"], 1);
    assert_eq!(f.get("Valid.md")["content"], "# Valid");
    assert_eq!(std::fs::read(f.notes.join("Legacy.md")).unwrap(), bytes);
}

#[test]
fn agent_queries_read_local_notes_without_a_code_index() {
    let f = Fixture::new();
    f.write("Personal/Tea note.md", "# Tea\nBrewing oolong\n");
    let sid = {
        let conn = f.app.db().conn.lock().unwrap();
        conn.execute("INSERT INTO projects(id,name,root_path,sort_order,collapsed,created_at) VALUES('p','Notebook',?1,0,0,0)",[f.notes.to_str().unwrap()]).unwrap();
        crate::db::repo::create_session(
            &conn,
            "p",
            None,
            "Notebook",
            crate::models::SessionKind::Terminal,
            None,
            Some(f.notes.to_str().unwrap()),
            None,
            None,
            None,
        )
        .unwrap()
        .id
    };
    let query = |action: &str, text: &str| {
        crate::knowledge::agent::query(
            &f.app,
            &json!({"sessionId":sid,"cwd":f.notes,"action":action,"query":text}),
        )
        .unwrap()
    };
    assert_eq!(
        query("notes", "oolong")["entries"][0]["path"],
        "Personal/Tea note.md"
    );
    assert_eq!(
        query("note", &format!("{} Personal/Tea note.md", f.id))["content"],
        "# Tea\nBrewing oolong\n"
    );
    assert_eq!(query("search", "oolong")["notes"]["total"], 1);
}

#[test]
fn reopening_a_notebook_preserves_favorites_and_trash_history() {
    let f = Fixture::new();
    f.write("Keep.md", "Keep");
    f.write("Restore.md", "Restore");
    f.call("kb_favorite", json!({"path":"Keep.md","favorite":true}))
        .unwrap();
    let removed = f.call("kb_trash", json!({"path":"Restore.md"})).unwrap();
    f.call("kb_unregister", json!({})).unwrap();
    let reopened = dispatch(&f.app, "kb_register", &json!({"root":f.notes})).unwrap();
    assert_eq!(reopened["id"], f.id);
    assert_eq!(f.get("Keep.md")["favorite"], true);
    f.call("kb_restore", json!({"id":removed["id"]})).unwrap();
    assert_eq!(f.get("Restore.md")["content"], "Restore");
}

#[test]
fn import_history_keeps_outcomes_and_record_deletion_leaves_files() {
    let f = Fixture::new();
    f.call(
        "kb_create",
        json!({"parent":"","name":"Imported","kind":"folder"}),
    )
    .unwrap();
    let plan = f
        .call(
            "kb_import_begin",
            json!({"destination":"Imported","files":[
                {"path":"Note.md","size":5},
                {"path":"Pic.svg","size":3},
                {"path":".hidden.md","size":2}
            ]}),
        )
        .unwrap();
    assert_eq!(plan["skipped"], 1);
    let id = plan["importId"].as_str().unwrap().to_string();
    assert_eq!(
        f.call("kb_import_delete", json!({"importId":id})).unwrap_err(),
        "kb_conflict"
    );
    let running = f.call("kb_imports", json!({})).unwrap();
    assert_eq!(running["total"], 1);
    assert_eq!(running["imports"][0]["status"], "running");
    assert_eq!(running["imports"][0]["files"], 2);
    assert_eq!(running["imports"][0]["skipped"], 1);
    assert_eq!(running["imports"][0]["bytes"], 8);

    let chunk = |index: u64, body: &[u8]| {
        f.call(
            "kb_import_chunk",
            json!({"importId":id,"index":index,"offset":0,"base64":base64::engine::general_purpose::STANDARD.encode(body)}),
        )
        .unwrap()
    };
    chunk(0, b"hello");
    chunk(1, b"svg");
    assert_eq!(
        f.call("kb_import_commit", json!({"importId":id})).unwrap()["imported"],
        2
    );

    let completed = f.call("kb_imports", json!({})).unwrap();
    assert_eq!(completed["imports"][0]["status"], "completed");
    assert_eq!(completed["imports"][0]["imported"], 2);
    assert_eq!(completed["imports"][0]["skipped"], 1);
    assert!(completed["imports"][0]["finishedAt"].as_i64().unwrap() > 0);
    let detail = f.call("kb_import_detail", json!({"importId":id})).unwrap();
    let files = detail["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0]["path"], ".hidden.md");
    assert_eq!(files[0]["status"], "skipped");
    assert_eq!(files[0]["reason"], "hidden");
    assert_eq!(files[1]["path"], "Note.md");
    assert_eq!(files[1]["status"], "imported");
    assert_eq!(files[2]["path"], "Pic.svg");
    assert_eq!(files[2]["status"], "imported");

    f.call("kb_import_delete", json!({"importId":id})).unwrap();
    assert_eq!(f.call("kb_imports", json!({})).unwrap()["total"], 0);
    assert_eq!(
        f.call("kb_import_detail", json!({"importId":id})).unwrap_err(),
        "kb_not_found"
    );
    assert_eq!(
        std::fs::read(f.notes.join("Imported/Note.md")).unwrap(),
        b"hello"
    );
    assert_eq!(
        std::fs::read(f.notes.join("Imported/Pic.svg")).unwrap(),
        b"svg"
    );
}

#[test]
fn cancelled_failed_and_abandoned_imports_close_their_records() {
    let f = Fixture::new();
    let record = |id: &str| {
        f.call("kb_imports", json!({}))
            .unwrap()["imports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == json!(id))
            .cloned()
            .unwrap()
    };

    let plan = f
        .call("kb_import_begin", json!({"files":[{"path":"a.md","size":1}]}))
        .unwrap();
    let cancelled = plan["importId"].as_str().unwrap().to_string();
    f.call("kb_import_abort", json!({"importId":cancelled}))
        .unwrap();
    assert_eq!(record(&cancelled)["status"], "cancelled");
    let detail = f
        .call("kb_import_detail", json!({"importId":cancelled}))
        .unwrap();
    assert_eq!(detail["files"][0]["status"], "pending");

    let plan = f
        .call("kb_import_begin", json!({"files":[{"path":"b.md","size":1}]}))
        .unwrap();
    let failed = plan["importId"].as_str().unwrap().to_string();
    f.write("b.md", "occupied");
    assert_eq!(
        f.call("kb_import_commit", json!({"importId":failed}))
            .unwrap_err(),
        "kb_exists"
    );
    let failed = record(&failed);
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["error"], "kb_exists");
    // An abort after a failed publish must not downgrade the record.
    f.call("kb_import_abort", json!({"importId":failed["id"]}))
        .unwrap();
    assert_eq!(record(failed["id"].as_str().unwrap())["status"], "failed");

    // An abandoned client stops refreshing its heartbeat; the next read retires the record.
    let plan = f
        .call("kb_import_begin", json!({"files":[{"path":"c.md","size":1}]}))
        .unwrap();
    let abandoned = plan["importId"].as_str().unwrap().to_string();
    {
        let conn = f.app.db().conn.lock().unwrap();
        conn.execute(
            "UPDATE kb_imports SET updated_at=1 WHERE id=?1",
            [&abandoned],
        )
        .unwrap();
    }
    let entry = record(&abandoned);
    assert_eq!(entry["status"], "interrupted");
    assert_eq!(entry["error"], "kb_interrupted");
    let stage = f.notes.join(".vkb-import").join(&abandoned);
    assert!(stage.is_dir());
    f.call("kb_import_delete", json!({"importId":abandoned}))
        .unwrap();
    assert!(!stage.is_dir());
}

#[test]
fn renaming_a_vault_follows_the_folder_and_rejects_collisions() {
    let f = Fixture::new();
    f.write("Note.md", "# Note");
    let renamed = f.call("kb_vault_rename", json!({"name":"Journal"})).unwrap();
    assert_eq!(renamed["name"], "Journal");
    assert!(f.notes.parent().unwrap().join("Journal").join("Note.md").is_file());
    assert!(!f.notes.exists());
    let overview = dispatch(&f.app, "kb_overview", &json!({})).unwrap();
    assert_eq!(overview["vaults"][0]["name"], "Journal");
    assert_eq!(overview["vaults"][0]["root"], renamed["root"]);

    std::fs::create_dir_all(f.notes.parent().unwrap().join("Taken")).unwrap();
    assert_eq!(f.call("kb_vault_rename", json!({"name":"Taken"})).unwrap_err(), "kb_exists");
    assert_eq!(f.call("kb_vault_rename", json!({"name":"nested/name"})).unwrap_err(), "kb_invalid_path");
    assert_eq!(f.call("kb_vault_rename", json!({"name":".hidden"})).unwrap_err(), "kb_invalid_path");
    // The failed attempts left the registered folder untouched.
    assert!(f.notes.parent().unwrap().join("Journal").join("Note.md").is_file());
}
