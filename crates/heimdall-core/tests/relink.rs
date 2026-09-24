//! Following a move with the links that pointed at it (SPEC §15, §17).
//!
//! The rule these tests exist to hold is narrow and unforgiving: a rewritten
//! link must resolve to the note that moved, and everything else in the file
//! must come out byte-identical. A rewriter that is merely usually right
//! corrupts notes, and a vault is the one place a user cannot check every file.

use camino::Utf8PathBuf;
use heimdall_core::commands::{move_path, relink, MovePathRequest, RelinkRequest, RelinkResponse};
use heimdall_core::paths::RelPath;
use heimdall_core::Vault;

fn vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_data_dir(&root, &test_data_dir()).unwrap();
    (dir, vault)
}

fn note(vault: &Vault, path: &str, content: &str) {
    let path = RelPath::parse(path).unwrap();
    let parent = path.parent();
    if !parent.is_root() {
        vault.create_dir_all(&parent).unwrap();
    }
    vault.atomic_write(&path, content.as_bytes()).unwrap();
}

fn read(vault: &Vault, path: &str) -> String {
    String::from_utf8(vault.read(&RelPath::parse(path).unwrap()).unwrap()).unwrap()
}

/// Move, then follow it — the sequence the desktop performs.
fn move_and_relink(vault: &Vault, from: &str, to: &str) -> RelinkResponse {
    move_path(
        vault,
        MovePathRequest {
            from: from.to_string(),
            to: to.to_string(),
        },
    )
    .unwrap();
    relink(
        vault,
        RelinkRequest {
            from: from.to_string(),
            to: to.to_string(),
            dry_run: false,
        },
    )
    .unwrap()
}

/* The shapes a link can be written in ---------------------------------- */

#[test]
fn a_bare_wikilink_follows_a_rename_and_stays_bare() {
    let (_dir, vault) = vault();
    note(&vault, "projects/roadmap.md", "# Roadmap\n");
    note(&vault, "ideas/source.md", "See [[roadmap]] for dates.\n");

    let response = move_and_relink(&vault, "projects/roadmap.md", "projects/plan.md");

    assert_eq!(read(&vault, "ideas/source.md"), "See [[plan]] for dates.\n");
    assert_eq!(response.updated.len(), 1);
    assert_eq!(response.updated[0].links, 1);
    assert!(response.skipped.is_empty());
}

#[test]
fn a_vault_relative_wikilink_keeps_its_full_path() {
    let (_dir, vault) = vault();
    note(&vault, "projects/roadmap.md", "# Roadmap\n");
    note(&vault, "ideas/source.md", "See [[projects/roadmap]].\n");

    move_and_relink(&vault, "projects/roadmap.md", "projects/plan.md");

    assert_eq!(read(&vault, "ideas/source.md"), "See [[projects/plan]].\n");
}

#[test]
fn a_note_relative_wikilink_is_rewritten_from_where_the_linking_note_sits() {
    let (_dir, vault) = vault();
    note(&vault, "projects/roadmap.md", "# Roadmap\n");
    note(&vault, "projects/deep/source.md", "See [[../roadmap]].\n");

    move_and_relink(&vault, "projects/roadmap.md", "projects/plan.md");

    assert_eq!(
        read(&vault, "projects/deep/source.md"),
        "See [[../plan]].\n"
    );
}

#[test]
fn an_extension_is_kept_only_when_the_link_carried_one() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(&vault, "source.md", "[[roadmap.md]] and [[roadmap]]\n");

    move_and_relink(&vault, "roadmap.md", "plan.md");

    assert_eq!(read(&vault, "source.md"), "[[plan.md]] and [[plan]]\n");
}

#[test]
fn an_alias_an_anchor_and_an_embed_survive_the_rewrite() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(
        &vault,
        "source.md",
        "[[roadmap|the plan]] [[roadmap#Q3]] [[roadmap#^ref]] ![[roadmap]]\n",
    );

    move_and_relink(&vault, "roadmap.md", "plan.md");

    assert_eq!(
        read(&vault, "source.md"),
        "[[plan|the plan]] [[plan#Q3]] [[plan#^ref]] ![[plan]]\n"
    );
}

#[test]
fn a_markdown_link_is_rewritten_relative_to_the_linking_note() {
    let (_dir, vault) = vault();
    note(&vault, "projects/roadmap.md", "# Roadmap\n");
    note(
        &vault,
        "projects/source.md",
        "[the plan](roadmap.md) and [again](./roadmap.md)\n",
    );

    move_and_relink(&vault, "projects/roadmap.md", "projects/plan.md");

    assert_eq!(
        read(&vault, "projects/source.md"),
        "[the plan](plan.md) and [again](plan.md)\n"
    );
}

#[test]
fn a_markdown_title_and_angle_brackets_are_left_alone() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(
        &vault,
        "source.md",
        "[a](roadmap.md \"The roadmap\") [b](<roadmap.md>)\n",
    );

    move_and_relink(&vault, "roadmap.md", "my plan.md");

    assert_eq!(
        read(&vault, "source.md"),
        "[a](my%20plan.md \"The roadmap\") [b](<my plan.md>)\n"
    );
}

#[test]
fn a_percent_encoded_markdown_link_follows_the_note_it_named() {
    let (_dir, vault) = vault();
    note(&vault, "my note.md", "# My note\n");
    note(&vault, "source.md", "[x](my%20note.md)\n");

    move_and_relink(&vault, "my note.md", "other note.md");

    assert_eq!(read(&vault, "source.md"), "[x](other%20note.md)\n");
}

/* What must never be touched ------------------------------------------- */

#[test]
fn links_in_fenced_and_inline_code_are_not_rewritten() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    let source = "Real [[roadmap]].\n\n```mermaid\ngraph TD\n  A[\"[[roadmap]]\"]\n```\n\nType `[[roadmap]]` to link.\n";
    note(&vault, "source.md", source);

    move_and_relink(&vault, "roadmap.md", "plan.md");

    assert_eq!(
        read(&vault, "source.md"),
        "Real [[plan]].\n\n```mermaid\ngraph TD\n  A[\"[[roadmap]]\"]\n```\n\nType `[[roadmap]]` to link.\n"
    );
}

#[test]
fn a_link_in_real_frontmatter_is_rewritten_and_one_under_a_rule_is_not() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(
        &vault,
        "real.md",
        "---\nlinks:\n  - \"[[roadmap]]\"\n---\n\n# Real\n",
    );
    // An unterminated block is a horizontal rule, not YAML — and the scanner
    // treats what follows as body, so the link is still a link.
    note(&vault, "rule.md", "---\nnot really yaml\n\n[[roadmap]]\n");

    move_and_relink(&vault, "roadmap.md", "plan.md");

    assert_eq!(
        read(&vault, "real.md"),
        "---\nlinks:\n  - \"[[plan]]\"\n---\n\n# Real\n"
    );
    assert_eq!(
        read(&vault, "rule.md"),
        "---\nnot really yaml\n\n[[plan]]\n"
    );
}

#[test]
fn a_note_with_no_link_to_the_moved_path_is_never_written() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(&vault, "other.md", "# Other\n\nSee [[roadmap_notes]].\n");
    note(&vault, "roadmap_notes.md", "# Notes\n");

    let response = move_and_relink(&vault, "roadmap.md", "plan.md");

    // `[[roadmap_notes]]` names a different note and must not be caught by a
    // prefix or substring match on the one that moved.
    assert_eq!(
        read(&vault, "other.md"),
        "# Other\n\nSee [[roadmap_notes]].\n"
    );
    assert!(response.updated.is_empty());
}

#[test]
fn a_note_in_the_trash_is_left_where_it_is() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(&vault, ".trash/old.md", "See [[roadmap]].\n");

    move_and_relink(&vault, "roadmap.md", "plan.md");

    assert_eq!(read(&vault, ".trash/old.md"), "See [[roadmap]].\n");
}

/* Ambiguity, which is where a naive rewriter goes wrong ------------------ */

#[test]
fn a_rename_into_an_ambiguous_basename_escalates_to_the_full_path() {
    let (_dir, vault) = vault();
    note(&vault, "projects/roadmap.md", "# Roadmap\n");
    // Renaming to `plan` would make a bare `[[plan]]` name this one instead:
    // it is shallower, and shallowest wins the tie-break.
    note(&vault, "plan.md", "# The other plan\n");
    note(&vault, "ideas/source.md", "See [[roadmap]].\n");

    let response = move_and_relink(&vault, "projects/roadmap.md", "projects/plan.md");

    assert_eq!(
        read(&vault, "ideas/source.md"),
        "See [[projects/plan]].\n",
        "a bare name would have pointed at the root note"
    );
    assert!(response.skipped.is_empty());
}

#[test]
fn a_move_that_never_happened_rewrites_nothing() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(&vault, "source.md", "See [[roadmap]].\n");

    // `to` does not exist, so nothing in the vault reads as having moved and
    // no link is a candidate. A stale call from the client is inert, not
    // destructive.
    let response = relink(
        &vault,
        RelinkRequest {
            from: "roadmap.md".to_string(),
            to: "nowhere.md".to_string(),
            dry_run: false,
        },
    )
    .unwrap();

    assert_eq!(read(&vault, "source.md"), "See [[roadmap]].\n");
    assert!(response.updated.is_empty());
    assert!(response.skipped.is_empty());
}

#[test]
fn a_note_past_the_per_file_link_cap_is_left_whole_and_counted() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    // `link_graph` truncates such a file and still draws. A rewrite cannot:
    // moving the first thousand links and leaving the rest would be a note in
    // two states, reported as if it were in one.
    let crowded = "[[roadmap]]\n".repeat(1_001);
    note(&vault, "crowded.md", &crowded);

    let response = move_and_relink(&vault, "roadmap.md", "plan.md");

    assert_eq!(read(&vault, "crowded.md"), crowded, "not partly rewritten");
    assert!(response.updated.is_empty());
    assert_eq!(response.truncated.files_unscanned, 1);
}

/* Folders, and the moved note's own links ------------------------------- */

#[test]
fn a_folder_rename_carries_every_link_into_it() {
    let (_dir, vault) = vault();
    note(&vault, "projects/one.md", "# One\n");
    note(&vault, "projects/two.md", "# Two\n");
    note(
        &vault,
        "source.md",
        "[[projects/one]] and [[projects/two]] and [[one]]\n",
    );

    let response = move_and_relink(&vault, "projects", "plans");

    assert_eq!(
        read(&vault, "source.md"),
        // The bare `[[one]]` still names the same note, so only the two paths
        // change — and the file is written once, not twice.
        "[[plans/one]] and [[plans/two]] and [[one]]\n"
    );
    assert_eq!(response.updated.len(), 1);
    assert_eq!(response.updated[0].links, 2);
}

#[test]
fn a_moved_notes_own_relative_links_are_re_expressed_from_its_new_home() {
    let (_dir, vault) = vault();
    note(&vault, "projects/deep/source.md", "See [[../sibling]].\n");
    note(&vault, "projects/sibling.md", "# Sibling\n");

    move_and_relink(&vault, "projects/deep/source.md", "source.md");

    assert_eq!(read(&vault, "source.md"), "See [[projects/sibling]].\n");
}

/* Locks ----------------------------------------------------------------- */

#[test]
fn a_locked_note_keeps_its_links_and_is_named_instead() {
    let (_dir, vault) = vault();
    note(&vault, "projects/roadmap.md", "# Roadmap\n");
    note(&vault, "kept/index.md", "- [[roadmap]]\n");
    note(&vault, "open.md", "- [[roadmap]]\n");
    heimdall_core::lock(&vault, heimdall_core::LockRequest { path: Some("kept".into()) }).unwrap();

    let response = move_and_relink(&vault, "projects/roadmap.md", "projects/plan.md");

    assert_eq!(read(&vault, "kept/index.md"), "- [[roadmap]]\n", "a lock is never written through");
    assert_eq!(read(&vault, "open.md"), "- [[plan]]\n");
    assert_eq!(response.locked, ["kept/index.md"]);
}

/* Contract -------------------------------------------------------------- */

#[test]
fn a_dry_run_reports_the_same_work_and_writes_nothing() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(&vault, "source.md", "See [[roadmap]].\n");
    move_path(
        &vault,
        MovePathRequest {
            from: "roadmap.md".into(),
            to: "plan.md".into(),
        },
    )
    .unwrap();

    let request = |dry_run| RelinkRequest {
        from: "roadmap.md".into(),
        to: "plan.md".into(),
        dry_run,
    };
    let dry = relink(&vault, request(true)).unwrap();
    assert_eq!(read(&vault, "source.md"), "See [[roadmap]].\n");

    let wet = relink(&vault, request(false)).unwrap();
    assert_eq!(read(&vault, "source.md"), "See [[plan]].\n");
    assert_eq!(dry.updated.len(), wet.updated.len());
    assert_eq!(dry.updated[0].path, wet.updated[0].path);
    assert_eq!(dry.updated[0].links, wet.updated[0].links);
    assert_eq!(
        dry.updated[0].new_revision.as_str(),
        wet.updated[0].new_revision.as_str(),
        "a dry run reports the revision the real write produces"
    );
}

#[test]
fn moving_nowhere_is_a_caller_mistake() {
    let (_dir, vault) = vault();
    let error = relink(
        &vault,
        RelinkRequest {
            from: "a.md".into(),
            to: "a.md".into(),
            dry_run: false,
        },
    )
    .unwrap_err();
    assert_eq!(error.code.as_str(), "INVALID_INPUT");
}

#[test]
fn a_second_run_over_a_settled_vault_changes_nothing() {
    let (_dir, vault) = vault();
    note(&vault, "roadmap.md", "# Roadmap\n");
    note(&vault, "source.md", "See [[roadmap]] and [[roadmap|it]].\n");

    move_and_relink(&vault, "roadmap.md", "plan.md");
    let settled = read(&vault, "source.md");

    let again = relink(
        &vault,
        RelinkRequest {
            from: "roadmap.md".into(),
            to: "plan.md".into(),
            dry_run: false,
        },
    )
    .unwrap();

    assert!(again.updated.is_empty(), "the work is idempotent");
    assert_eq!(read(&vault, "source.md"), settled);
}

fn test_data_dir() -> camino::Utf8PathBuf {
    camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .expect("temp dir is UTF-8")
        .join("heimdall-test-data")
}
