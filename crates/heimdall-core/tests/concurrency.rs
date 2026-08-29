//! Contention on the write path (SPEC §14).
//!
//! These hammer the paths where several writers meet. They exist because
//! acquiring a lock used to open its sidecar with `O_CREAT` and no `O_EXCL`:
//! resolving a path one component at a time, as a directory capability must,
//! can report `NotFound` for a leaf another writer is creating at that very
//! instant, so a write would fail with a bogus `NOT_FOUND` under load.

use std::collections::HashSet;
use std::sync::Mutex;

use camino::Utf8PathBuf;
use heimdall_core::commands::{create_entry, CreateEntryRequest, EntryKind};
use heimdall_core::paths::{self, RelPath};
use heimdall_core::{template, Vault};

const THREADS: usize = 16;

fn scaffolded_vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_lock_dir(&root, &test_lock_dir()).unwrap();
    template::scaffold_full(&vault).unwrap();
    (dir, vault)
}

fn main_memory() -> RelPath {
    RelPath::parse(paths::MAIN_MEMORY_FILE).unwrap()
}

#[test]
fn many_writers_can_acquire_one_lock_that_does_not_exist_yet() {
    // Every round starts with no sidecar, so all threads race to create it.
    for round in 0..15 {
        let (_tmp, vault) = scaffolded_vault();
        let target = main_memory();

        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                let (vault, target) = (&vault, &target);
                scope.spawn(move || {
                    for iteration in 0..10 {
                        vault.with_write_lock(target, || Ok(())).unwrap_or_else(|err| {
                            panic!("round {round} iteration {iteration}: {} {}", err.code, err.message)
                        });
                    }
                });
            }
        });
    }
}

#[test]
fn the_lock_actually_excludes_concurrent_holders() {
    let (_tmp, vault) = scaffolded_vault();
    let target = main_memory();
    let inside = Mutex::new(0usize);
    let overlaps = Mutex::new(0usize);

    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            let (vault, target, inside, overlaps) = (&vault, &target, &inside, &overlaps);
            scope.spawn(move || {
                for _ in 0..10 {
                    vault
                        .with_write_lock(target, || {
                            {
                                let mut count = inside.lock().unwrap();
                                *count += 1;
                                if *count > 1 {
                                    *overlaps.lock().unwrap() += 1;
                                }
                            }
                            std::thread::yield_now();
                            *inside.lock().unwrap() -= 1;
                            Ok(())
                        })
                        .unwrap();
                }
            });
        }
    });

    assert_eq!(*overlaps.lock().unwrap(), 0, "two holders were inside the lock at once");
}

#[test]
fn locked_read_modify_write_never_loses_or_corrupts_a_write() {
    for round in 0..4 {
        let (_tmp, vault) = scaffolded_vault();
        let target = main_memory();

        std::thread::scope(|scope| {
            for index in 0..8 {
                let (vault, target) = (&vault, &target);
                scope.spawn(move || {
                    for iteration in 0..3 {
                        let body = format!("thread {index} iteration {iteration}\n");
                        vault
                            .with_write_lock(target, || {
                                vault.read(target)?;
                                vault.atomic_write(target, body.as_bytes())
                            })
                            .unwrap_or_else(|err| panic!("round {round}: {} {}", err.code, err.message));
                    }
                });
            }
        });

        // Whatever landed last is exactly one writer's content, not a blend.
        let stored = String::from_utf8(vault.read(&target).unwrap()).unwrap();
        assert_eq!(stored.lines().count(), 1, "content was interleaved: {stored:?}");
        assert!(stored.starts_with("thread "), "{stored:?}");
    }
}

#[test]
fn concurrent_entry_creation_gives_every_writer_its_own_file() {
    for round in 0..8 {
        let (_tmp, vault) = scaffolded_vault();
        let ids = Mutex::new(Vec::new());

        std::thread::scope(|scope| {
            for index in 0..THREADS {
                let (vault, ids) = (&vault, &ids);
                scope.spawn(move || {
                    let response = create_entry(
                        vault,
                        CreateEntryRequest {
                            kind: EntryKind::Conversation,
                            content: format!("body {index}\n"),
                        },
                    )
                    .unwrap_or_else(|err| panic!("round {round}: {} {}", err.code, err.message));
                    ids.lock().unwrap().push(response.id);
                });
            }
        });

        // Entries are create-only: `O_EXCL` hands each writer a distinct name
        // even when they all land in the same second.
        let ids = ids.into_inner().unwrap();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), THREADS, "round {round} ids collided: {ids:?}");
    }
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
