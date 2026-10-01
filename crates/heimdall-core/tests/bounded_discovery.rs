//! The listing scan guard (SPEC §8).
//!
//! One folder `read` examines at most 10,000 filesystem entries. When
//! the guard trips, the partial page still comes back with a cursor so the
//! caller can keep going rather than being stuck at a hard wall.

use camino::Utf8PathBuf;
use heimdall_core::commands::{read, Listing, ReadRequest};
use heimdall_core::limits::LIST_SCAN_GUARD;
use heimdall_core::Vault;

/// A vault holding more notes than one call is allowed to examine.
fn oversized_vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_data_dir(&root, &test_data_dir()).unwrap();

    let notes = dir.path().join("notes");
    std::fs::create_dir(&notes).unwrap();
    for index in 0..LIST_SCAN_GUARD + 100 {
        std::fs::write(notes.join(format!("note_{index:06}.md")), b"x").unwrap();
    }
    (dir, vault)
}

fn list(vault: &Vault, cursor: Option<String>) -> Listing {
    read(
        vault,
        ReadRequest {
            vault: None,
            recursive: true,
            cursor,
            limit: Some(200),
            ..Default::default()
        },
    )
    .unwrap()
    .listing
    .expect("the vault root is a folder")
}

#[test]
fn the_scan_guard_returns_a_partial_page_rather_than_reading_the_whole_vault() {
    let (_tmp, vault) = oversized_vault();
    let response = list(&vault, None);

    assert!(response.scan_guard_hit, "guard should have tripped");
    assert!(
        !response.entries.is_empty(),
        "a tripped guard must still return progress"
    );
    assert!(
        response.next_cursor.is_some(),
        "a tripped guard must offer a way to continue"
    );
}

#[test]
fn paging_past_the_scan_guard_makes_progress_and_terminates() {
    let (_tmp, vault) = oversized_vault();

    let mut seen: Vec<String> = Vec::new();
    let mut cursor = None;
    // Generous ceiling: the point is that it terminates well before this.
    for _ in 0..500 {
        let response = list(&vault, cursor.clone());
        assert!(
            response.entries.iter().all(|e| e.path.as_str() > cursor.as_deref().unwrap_or("")),
            "a page repeated ground already covered"
        );
        seen.extend(response.entries.iter().map(|e| e.path.clone()));
        cursor = response.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    assert!(cursor.is_none(), "listing never finished");

    // Every note was returned exactly once, in one total order.
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), seen.len(), "a document appeared on two pages");
    assert_eq!(seen.len(), LIST_SCAN_GUARD + 100 + 1, "expected every note plus notes/");

    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(seen, sorted, "pages did not follow one global order");
}

/// The graph's own bounds (SPEC §8).
///
/// `link_graph` is the one operation that reads the whole vault, so it is capped
/// rather than paginated. Every cap is a truncated success that says what it left
/// out — the same treatment the listing scan guard gets above, and the reason the
/// promise "no operation reads an entire vault without limits" still holds.
mod graph_bounds {
    use super::*;
    use heimdall_core::commands::{link_graph, LinkGraphRequest};
    use heimdall_core::limits::{GRAPH_MAX_FILE_BYTES, GRAPH_MAX_NODES};
    use heimdall_core::paths::RelPath;

    fn vault_with(count: usize) -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open_with_data_dir(&root, &test_data_dir()).unwrap();

        let notes = dir.path().join("notes");
        std::fs::create_dir(&notes).unwrap();
        for index in 0..count {
            std::fs::write(notes.join(format!("note_{index:06}.md")), b"# n\n").unwrap();
        }
        (dir, vault)
    }

    #[test]
    fn the_node_cap_truncates_and_reports_exactly_what_it_left_out() {
        let over = 40usize;
        let (_dir, vault) = vault_with(GRAPH_MAX_NODES + over);

        let response = link_graph(&vault, LinkGraphRequest::default()).unwrap();

        assert_eq!(response.nodes.len(), GRAPH_MAX_NODES);
        assert!(response.truncated.node_cap_hit);
        assert_eq!(
            response.nodes.len() + response.truncated.nodes_omitted,
            GRAPH_MAX_NODES + over
        );
    }

    #[test]
    fn a_note_larger_than_the_per_file_cap_is_a_node_without_edges() {
        let (dir, vault) = vault_with(1);
        let huge = "x".repeat(GRAPH_MAX_FILE_BYTES as usize + 1);
        std::fs::write(dir.path().join("notes/huge.md"), &huge).unwrap();

        let response = link_graph(&vault, LinkGraphRequest::default()).unwrap();
        let node = response
            .nodes
            .iter()
            .find(|node| node.path == "notes/huge.md")
            .expect("it is still a node");

        // Present in the graph, but `scanned: false` — the difference between
        // "this note has no links" and "we did not look".
        assert!(!node.scanned);
        assert_eq!(response.truncated.files_unscanned, 1);
        assert!(!response.truncated.total_bytes_cap_hit);
    }

    #[test]
    fn the_scan_budget_is_reported_so_a_client_knows_what_it_paid() {
        let (_dir, vault) = vault_with(10);
        let response = link_graph(&vault, LinkGraphRequest::default()).unwrap();

        assert!(response.truncated.scanned_bytes > 0);
        assert!(!response.truncated.total_bytes_cap_hit);
        assert_eq!(response.truncated.files_unscanned, 0);
        assert!(response.nodes.iter().all(|node| node.scanned));
    }

    #[test]
    fn depth_bounds_the_walk_the_same_way_a_listing_does() {
        let (dir, vault) = vault_with(0);
        let mut deep = dir.path().to_path_buf();
        for level in 0..6 {
            deep = deep.join(format!("level_{level}"));
            std::fs::create_dir(&deep).unwrap();
            std::fs::write(deep.join("note.md"), b"# n\n").unwrap();
        }

        let shallow = link_graph(
            &vault,
            LinkGraphRequest {
                max_depth: Some(2),
            },
        )
        .unwrap();
        assert_eq!(
            shallow
                .nodes
                .iter()
                .map(|node| node.path.clone())
                .collect::<Vec<_>>(),
            ["level_0/note.md"]
        );

        let full = link_graph(
            &vault,
            LinkGraphRequest {
                max_depth: Some(16),
            },
        )
        .unwrap();
        assert_eq!(full.nodes.len(), 6);
        // And the deepest one is reachable by its own name.
        assert!(RelPath::parse("level_0/level_1/level_2/level_3/level_4/level_5/note.md").is_ok());
    }
}

/// Where these tests keep Heimdall's application data (write locks, lock rules).
///
/// Outside the vault, as production does, but under the system temp directory
/// rather than the real application-data one: a test run must not leave files
/// in a developer's home. Per-vault files are named by a hash of the vault's
/// path and every vault here is a fresh temp directory, so sharing one
/// directory cannot collide.
fn test_data_dir() -> camino::Utf8PathBuf {
    camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .expect("temp dir is UTF-8")
        .join("heimdall-test-data")
}
