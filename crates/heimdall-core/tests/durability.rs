//! Atomic writes and recovery from an interrupted one (SPEC §14, §17).
//!
//! Writes land in a temporary sibling that is flushed before an atomic rename.
//! A process killed mid-write therefore leaves the previous file intact and, at
//! worst, an orphaned temp file — never a half-written note.

use camino::Utf8PathBuf;
use heimdall_core::commands::{list_documents, read_memory, ListDocumentsRequest, ReadMemoryRequest};
use heimdall_core::paths::{self, RelPath};
use heimdall_core::{template, Revision, Vault};

fn vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_lock_dir(&root, &test_lock_dir()).unwrap();
    template::scaffold_full(&vault).unwrap();
    (dir, vault)
}

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

/// Stand in for a process killed between writing a temp file and renaming it.
fn orphaned_temp(dir: &tempfile::TempDir, relative: &str, contents: &[u8]) {
    std::fs::write(dir.path().join(relative), contents).unwrap();
}

#[test]
fn an_interrupted_write_leaves_the_previous_content_readable() {
    let (dir, vault) = vault();
    let memory = rel(paths::MAIN_MEMORY_FILE);
    vault.atomic_write(&memory, b"# Memory\n\nCommitted.\n").unwrap();

    orphaned_temp(
        &dir,
        "aios/memories/.memory.md.tmp-99999-0",
        b"half-written garb",
    );

    let result = read_memory(&vault, ReadMemoryRequest::default()).unwrap();
    assert_eq!(result.content, "# Memory\n\nCommitted.\n");
    assert_eq!(result.revision, Revision::of_bytes(b"# Memory\n\nCommitted.\n"));
}

#[test]
fn an_orphaned_temp_file_is_never_mistaken_for_content() {
    let (dir, vault) = vault();
    vault.atomic_write(&rel("note.md"), b"real\n").unwrap();
    orphaned_temp(&dir, ".note.md.tmp-99999-0", b"garbage");
    orphaned_temp(&dir, ".note.md.lock", b"");

    let response = list_documents(
        &vault,
        ListDocumentsRequest {
            recursive: true,
            ..Default::default()
        },
    )
    .unwrap();

    let paths: Vec<_> = response.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"note.md"));
    assert!(
        !paths.iter().any(|p| p.contains(".tmp-") || p.ends_with(".lock")),
        "{paths:?}"
    );
}

#[test]
fn a_later_write_succeeds_despite_orphaned_temp_files() {
    let (dir, vault) = vault();
    let note = rel("note.md");
    vault.atomic_write(&note, b"first\n").unwrap();
    orphaned_temp(&dir, ".note.md.tmp-99999-0", b"garbage");

    let revision = vault.atomic_write(&note, b"second\n").unwrap();
    assert_eq!(vault.read(&note).unwrap(), b"second\n");
    assert_eq!(revision, Revision::of_bytes(b"second\n"));
}

#[test]
fn a_replacement_is_never_observable_as_a_partial_file() {
    // Every intermediate state of a replacement is either the old bytes or the
    // new ones, so a concurrent reader can only ever see a whole revision.
    let (_tmp, vault) = vault();
    let note = rel("note.md");
    let old = b"a".repeat(64 * 1024);
    let new = b"b".repeat(64 * 1024);

    vault.atomic_write(&note, &old).unwrap();
    let before = vault.read(&note).unwrap();
    vault.atomic_write(&note, &new).unwrap();
    let after = vault.read(&note).unwrap();

    assert_eq!(before, old);
    assert_eq!(after, new);
    assert!(after.iter().all(|byte| *byte == b'b'), "content was mixed");
}

#[test]
fn the_revision_is_computed_from_the_bytes_actually_stored() {
    let (_tmp, vault) = vault();
    let note = rel("note.md");

    // Content whose byte length differs from its character count, so a revision
    // taken over anything but the exact stored bytes would disagree.
    let content = "# Título\n\n🌍 world\n".as_bytes();
    let reported = vault.atomic_write(&note, content).unwrap();

    let on_disk = vault.read(&note).unwrap();
    assert_eq!(on_disk, content);
    assert_eq!(reported, Revision::of_bytes(&on_disk));
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
