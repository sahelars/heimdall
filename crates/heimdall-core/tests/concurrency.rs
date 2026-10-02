//! Contention on the write path (SPEC §14).
//!
//! These hammer the paths where several writers meet. They exist because
//! acquiring a lock used to open its sidecar with `O_CREAT` and no `O_EXCL`:
//! resolving a path one component at a time, as a directory capability must,
//! can report `NotFound` for a leaf another writer is creating at that very
//! instant, so a write would fail with a bogus `NOT_FOUND` under load.

use std::sync::Mutex;

use camino::Utf8PathBuf;
use heimdall_core::commands::{lock, write, LockRequest, WriteRequest};
use heimdall_core::paths::RelPath;
use heimdall_core::{template, ErrorCode, Vault};

const THREADS: usize = 16;

fn scaffolded_vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_data_dir(&root, &test_data_dir()).unwrap();
    template::scaffold_full(&vault).unwrap();
    (dir, vault)
}

fn target_note() -> RelPath {
    RelPath::parse("ideas/hello_world.md").unwrap()
}

#[test]
fn many_writers_can_acquire_one_lock_that_does_not_exist_yet() {
    // Every round starts with no sidecar, so all threads race to create it.
    for round in 0..15 {
        let (_tmp, vault) = scaffolded_vault();
        let target = target_note();

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
    let target = target_note();
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
        let target = target_note();

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
fn concurrent_creates_of_one_note_leave_exactly_one_winner() {
    for round in 0..8 {
        let (_tmp, vault) = scaffolded_vault();
        let winners = Mutex::new(Vec::new());

        std::thread::scope(|scope| {
            for index in 0..THREADS {
                let (vault, winners) = (&vault, &winners);
                scope.spawn(move || {
                    let outcome = write(
                        vault,
                        WriteRequest {
                            vault: None,
                            path: "ideas/race.md".to_string(),
                            content: format!("body {index}\n"),
                            expected_revision: None,
                        },
                    );
                    match outcome {
                        Ok(_) => winners.lock().unwrap().push(index),
                        Err(err) => assert_eq!(err.code, ErrorCode::RevisionConflict, "round {round}"),
                    }
                });
            }
        });

        // A create never replaces a note, so however many writers race for one
        // path, exactly one of them made it.
        let winners = winners.into_inner().unwrap();
        assert_eq!(winners.len(), 1, "round {round}: {winners:?}");
        let stored = String::from_utf8(vault.read(&RelPath::parse("ideas/race.md").unwrap()).unwrap()).unwrap();
        assert_eq!(stored, format!("body {}\n", winners[0]));
    }
}

#[test]
fn a_lock_racing_writers_is_never_half_applied() {
    // Every write either lands before the lock or is refused after it; none
    // lands after the lock was taken.
    let (_tmp, vault) = scaffolded_vault();
    let target = target_note();
    let revision = heimdall_core::Revision::of_bytes(&vault.read(&target).unwrap());

    std::thread::scope(|scope| {
        let vault = &vault;
        scope.spawn(move || {
            lock(vault, LockRequest { path: Some("ideas".to_string()) }).unwrap();
        });
        for index in 0..THREADS {
            let revision = revision.clone();
            scope.spawn(move || {
                let _ = write(
                    vault,
                    WriteRequest {
                        vault: None,
                        path: format!("ideas/new_{index}.md"),
                        content: "x\n".to_string(),
                        expected_revision: None,
                    },
                );
                let _ = write(
                    vault,
                    WriteRequest {
                        vault: None,
                        path: "ideas/hello_world.md".to_string(),
                        content: format!("{index}\n"),
                        expected_revision: Some(Some(revision)),
                    },
                );
            });
        }
    });

    let err = write(
        &vault,
        WriteRequest {
            vault: None,
            path: "ideas/after.md".to_string(),
            content: "x\n".to_string(),
            expected_revision: None,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
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
