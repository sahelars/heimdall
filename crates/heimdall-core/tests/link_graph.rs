//! The whole-vault link index (SPEC §8, §15, §17).
//!
//! Resolution follows the conventional wikilink rules, because a graph that
//! disagreed with what any other editor draws over the same files would be
//! worse than no graph.

use camino::Utf8PathBuf;
use heimdall_core::commands::{link_graph, LinkGraphRequest, LinkGraphResponse};
use heimdall_core::paths::RelPath;
use heimdall_core::{template, Vault};

fn vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_lock_dir(&root, &test_lock_dir()).unwrap();
    template::scaffold_aios_only(&vault).unwrap();
    (dir, vault)
}

/// Write a note, creating its folder first.
fn note(vault: &Vault, path: &str, content: &str) {
    let path = RelPath::parse(path).unwrap();
    let parent = path.parent();
    if !parent.is_root() {
        vault.create_dir_all(&parent).unwrap();
    }
    vault.atomic_write(&path, content.as_bytes()).unwrap();
}

fn graph(vault: &Vault) -> LinkGraphResponse {
    link_graph(vault, LinkGraphRequest::default()).unwrap()
}

/// Every edge as `("from/path.md", "to/path.md", count)`.
fn edges(response: &LinkGraphResponse) -> Vec<(String, String, usize)> {
    response
        .edges
        .iter()
        .map(|edge| {
            (
                response.nodes[edge.from].path.clone(),
                response.nodes[edge.to].path.clone(),
                edge.count,
            )
        })
        .collect()
}

fn paths(response: &LinkGraphResponse) -> Vec<String> {
    response.nodes.iter().map(|node| node.path.clone()).collect()
}

#[test]
fn a_bare_wikilink_resolves_to_the_note_with_that_basename() {
    let (_dir, vault) = vault();
    note(&vault, "ideas/source.md", "See [[target]].\n");
    note(&vault, "projects/target.md", "# Target\n");

    assert_eq!(
        edges(&graph(&vault)),
        [("ideas/source.md".into(), "projects/target.md".into(), 1)]
    );
}

#[test]
fn the_title_is_the_filename_stem_which_is_what_a_graph_shows() {
    let (_dir, vault) = vault();
    note(&vault, "projects/lens/how_lens_works.md", "# H\n");

    let response = graph(&vault);
    let node = response
        .nodes
        .iter()
        .find(|node| node.path == "projects/lens/how_lens_works.md")
        .expect("the note is a node");
    assert_eq!(node.title, "how_lens_works");
}

#[test]
fn an_ambiguous_basename_resolves_to_the_shortest_path() {
    let (_dir, vault) = vault();
    note(&vault, "start.md", "Go to [[target]].\n");
    note(&vault, "target.md", "shallow\n");
    note(&vault, "a/b/c/target.md", "deep\n");

    assert_eq!(
        edges(&graph(&vault)),
        [("start.md".into(), "target.md".into(), 1)]
    );
}

#[test]
fn a_folder_qualified_link_beats_the_basename_match() {
    let (_dir, vault) = vault();
    note(&vault, "start.md", "Go to [[a/b/c/target]].\n");
    note(&vault, "target.md", "shallow\n");
    note(&vault, "a/b/c/target.md", "deep\n");

    assert_eq!(
        edges(&graph(&vault)),
        [("start.md".into(), "a/b/c/target.md".into(), 1)]
    );
}

#[test]
fn every_wikilink_spelling_of_one_target_collapses_into_a_counted_edge() {
    let (_dir, vault) = vault();
    note(
        &vault,
        "ideas/source.md",
        "[[target]] [[target|alias]] [[target#heading]] [[target#^block]] [[ideas/target]]\n",
    );
    note(&vault, "ideas/target.md", "# Target\n");

    assert_eq!(
        edges(&graph(&vault)),
        [("ideas/source.md".into(), "ideas/target.md".into(), 5)]
    );
}

#[test]
fn links_inside_fenced_code_are_not_edges() {
    let (_dir, vault) = vault();
    // Exactly the shape of the mermaid block in the reference screenshots.
    note(
        &vault,
        "ideas/source.md",
        "```mermaid\ngraph TD\n  A[\"[[target]]\"] --> B\n```\n",
    );
    note(&vault, "ideas/target.md", "# Target\n");

    let response = graph(&vault);
    assert!(edges(&response).is_empty());
    assert!(response.unresolved.is_empty());
}

#[test]
fn frontmatter_links_are_edges() {
    let (_dir, vault) = vault();
    note(
        &vault,
        "projects/lens/how_lens_works.md",
        "---\nlinks:\n  - \"[[profile]]\"\n  - \"[[articles]]\"\n---\n\n# How it works\n",
    );
    note(&vault, "projects/lens/profile.md", "# Profile\n");
    note(&vault, "projects/lens/articles.md", "# Articles\n");

    let response = graph(&vault);
    let from = "projects/lens/how_lens_works.md".to_string();
    assert_eq!(
        edges(&response),
        [
            (from.clone(), "projects/lens/articles.md".into(), 1),
            (from, "projects/lens/profile.md".into(), 1),
        ]
    );
}

#[test]
fn a_markdown_link_resolves_relative_to_the_linking_note() {
    let (_dir, vault) = vault();
    note(&vault, "projects/lens/source.md", "See [target](target.md).\n");
    note(&vault, "projects/lens/target.md", "# Target\n");
    // A decoy with the same basename nearer the root: a Markdown link is a
    // path, so it must not be found by name.
    note(&vault, "target.md", "decoy\n");

    assert_eq!(
        edges(&graph(&vault)),
        [(
            "projects/lens/source.md".into(),
            "projects/lens/target.md".into(),
            1
        )]
    );
}

#[test]
fn a_markdown_link_is_never_resolved_by_basename() {
    let (_dir, vault) = vault();
    note(&vault, "a/source.md", "See [x](target.md).\n");
    note(&vault, "b/target.md", "# Target\n");

    let response = graph(&vault);
    assert!(edges(&response).is_empty());
    assert_eq!(response.unresolved[0].target, "target.md");
}

#[test]
fn an_unresolvable_target_is_reported_rather_than_dropped() {
    let (_dir, vault) = vault();
    note(&vault, "ideas/source.md", "[[nowhere]] and [[nowhere]] again\n");

    let response = graph(&vault);
    assert!(edges(&response).is_empty());
    assert_eq!(response.unresolved.len(), 1);
    assert_eq!(response.unresolved[0].target, "nowhere");
    // Counted, so the client can size an unresolved node by how often it is named.
    assert_eq!(response.unresolved[0].count, 2);
}

#[test]
fn a_self_link_is_not_an_edge() {
    let (_dir, vault) = vault();
    note(&vault, "ideas/source.md", "I am [[source]].\n");

    assert!(edges(&graph(&vault)).is_empty());
}

#[test]
fn the_graph_covers_the_protected_tree_and_marks_those_nodes() {
    let (_dir, vault) = vault();
    note(&vault, "ideas/source.md", "Remember [[memory]].\n");

    let response = graph(&vault);
    let memory = response
        .nodes
        .iter()
        .find(|node| node.path == "aios/memories/memory.md")
        .expect("the main memory is a node");
    assert!(memory.in_aios);

    // The file tree in the desktop shows aios/, so the graph has to agree.
    assert_eq!(
        edges(&response),
        [("ideas/source.md".into(), "aios/memories/memory.md".into(), 1)]
    );
}

#[test]
fn excluding_the_protected_tree_removes_its_nodes_and_every_edge_touching_them() {
    let (_dir, vault) = vault();
    note(&vault, "ideas/source.md", "Remember [[memory]].\n");

    let response = link_graph(
        &vault,
        LinkGraphRequest {
            include_aios: false,
            max_depth: None,
        },
    )
    .unwrap();

    assert_eq!(paths(&response), ["ideas/source.md"]);
    assert!(response.edges.is_empty());
    // Not silently dropped: the link is still reported, just unresolved.
    assert_eq!(response.unresolved[0].target, "memory");
}

#[test]
fn hidden_folders_and_debris_are_never_nodes() {
    let (dir, vault) = vault();
    note(&vault, "ideas/real.md", "# Real\n");
    std::fs::create_dir_all(dir.path().join(".trash/ideas")).unwrap();
    std::fs::write(dir.path().join(".trash/ideas/deleted.md"), "gone\n").unwrap();
    std::fs::write(dir.path().join("ideas/.DS_Store"), b"junk").unwrap();
    std::fs::write(dir.path().join("ideas/notes.txt"), "not markdown\n").unwrap();

    // `aios/memories/memory.md` is real content and belongs in the graph;
    // `.trash/`, `.DS_Store`, and a non-Markdown file do not.
    assert_eq!(
        paths(&graph(&vault)),
        ["aios/memories/memory.md", "ideas/real.md"]
    );
}

#[test]
fn a_note_that_is_not_valid_utf8_is_a_node_without_edges() {
    let (dir, vault) = vault();
    note(&vault, "ideas/good.md", "# Good\n");
    std::fs::write(dir.path().join("ideas/broken.md"), [0xff, 0xfe, 0x00]).unwrap();

    let response = graph(&vault);
    let broken = response
        .nodes
        .iter()
        .find(|node| node.path == "ideas/broken.md")
        .expect("it is still a node");
    // One unreadable file must not fail the whole index, and `scanned: false`
    // is the difference between "no links" and "we do not know".
    assert!(!broken.scanned);
    assert_eq!(response.truncated.files_unscanned, 1);
}

#[test]
fn the_graph_is_byte_for_byte_identical_across_runs() {
    let (_dir, vault) = vault();
    // Several notes sharing a basename, which is where an unordered map's
    // iteration order would otherwise leak into the output.
    for folder in ["a", "b", "c", "d"] {
        note(&vault, &format!("{folder}/shared.md"), "# Shared\n");
        note(&vault, &format!("{folder}/source.md"), "[[shared]] [[missing]]\n");
    }

    let first = serde_json::to_string(&graph(&vault)).unwrap();
    let second = serde_json::to_string(&graph(&vault)).unwrap();
    assert_eq!(first, second);
}

#[test]
fn the_response_carries_no_file_content() {
    let (_dir, vault) = vault();
    note(&vault, "ideas/secret.md", "The passphrase is hunter2.\n");

    let serialized = serde_json::to_string(&graph(&vault)).unwrap();
    // The whole basis for letting this operation see the protected tree.
    assert!(!serialized.contains("hunter2"));
    assert!(!serialized.contains("passphrase"));
}

#[test]
fn the_whole_vault_means_the_whole_vault_by_default() {
    let (_dir, vault) = vault();
    // Deeper than a listing's default depth of four. A graph that quietly
    // stopped there would report `deep` as unresolved rather than admit it had
    // not looked — and the client would draw a broken link over a real one.
    note(&vault, "a/b/c/mid.md", "Points at [[deep]].\n");
    note(&vault, "a/b/c/d/e/deep.md", "# Deep\n");

    let response = graph(&vault);

    assert!(paths(&response).iter().any(|path| path == "a/b/c/d/e/deep.md"));
    assert_eq!(
        edges(&response),
        [("a/b/c/mid.md".into(), "a/b/c/d/e/deep.md".into(), 1)]
    );
    assert!(response.unresolved.is_empty());
}

#[test]
fn a_relative_markdown_link_can_climb_out_of_its_folder() {
    let (_dir, vault) = vault();
    // An editor set to write relative paths produces links like this, and a
    // vault written that way would otherwise look entirely unlinked.
    note(&vault, "projects/lens/source.md", "See [target](../heimdall/target.md).\n");
    note(&vault, "projects/heimdall/target.md", "# Target\n");

    assert_eq!(
        edges(&graph(&vault)),
        [(
            "projects/lens/source.md".into(),
            "projects/heimdall/target.md".into(),
            1
        )]
    );
}

#[test]
fn a_relative_link_climbing_past_the_vault_root_is_unresolved_not_a_crash() {
    let (_dir, vault) = vault();
    note(&vault, "ideas/source.md", "See [x](../../../etc/passwd.md).\n");

    let response = graph(&vault);
    assert!(response.edges.is_empty());
    assert_eq!(response.unresolved[0].target, "../../../etc/passwd.md");
}

/// Where these tests keep their write locks.
///
/// Outside the vault, as production does, but under the system temp directory
/// rather than the real application-data one: a test run must not leave files
/// in a developer's home. Lock files are named by a hash of the vault's path
/// and every vault here is a fresh temp directory, so sharing one directory
/// cannot collide.
fn test_lock_dir() -> camino::Utf8PathBuf {
    camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .expect("temp dir is UTF-8")
        .join("heimdall-test-locks")
}
