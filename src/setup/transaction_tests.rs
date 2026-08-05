use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;

use super::transaction::Transaction;

#[test]
fn injected_failure_restores_exact_bytes_modes_and_missing_paths() {
    let home = tempfile_path("rollback");
    fs::create_dir_all(&home).unwrap();
    let first = home.join("first");
    let second = home.join("second");
    let created = home.join("created");
    fs::write(&first, b"first-before").unwrap();
    fs::write(&second, b"second-before").unwrap();
    fs::set_permissions(&first, Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(&second, Permissions::from_mode(0o604)).unwrap();

    let mut transaction = Transaction::new();
    transaction
        .write(&first, b"first-after".to_vec(), 0o600)
        .unwrap();
    transaction.remove_file(&second).unwrap();
    transaction
        .write(&created, b"created".to_vec(), 0o600)
        .unwrap();
    let failure = transaction
        .commit_with(|index| {
            if index == 2 {
                Err(io::Error::other("injected mutation failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(failure.rollback_failures.is_empty());
    assert_eq!(fs::read(&first).unwrap(), b"first-before");
    assert_eq!(fs::read(&second).unwrap(), b"second-before");
    assert!(!created.exists());
    assert_eq!(mode(&first), 0o640);
    assert_eq!(mode(&second), 0o604);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn failed_write_after_apply_restores_its_path_and_all_prior_mutations() {
    let home = tempfile_path("write-after-apply-rollback");
    fs::create_dir_all(&home).unwrap();
    let updated = home.join("updated");
    let removed = home.join("removed");
    let created = home.join("created");
    let failed = home.join("failed");
    fs::write(&updated, b"updated-before").unwrap();
    fs::write(&removed, b"removed-before").unwrap();
    fs::write(&failed, b"failed-before").unwrap();
    fs::set_permissions(&updated, Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(&removed, Permissions::from_mode(0o604)).unwrap();
    fs::set_permissions(&failed, Permissions::from_mode(0o640)).unwrap();

    let mut transaction = Transaction::new();
    transaction
        .write(&updated, b"updated-after".to_vec(), 0o600)
        .unwrap();
    transaction.remove_file(&removed).unwrap();
    transaction
        .write(&created, b"created-after".to_vec(), 0o600)
        .unwrap();
    transaction
        .write(&failed, b"failed-after".to_vec(), 0o600)
        .unwrap();
    let failure = transaction
        .commit_with_after_apply(|index| {
            if index == 3 {
                Err(io::Error::other("injected write failure after apply"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(failure.rollback_failures.is_empty());
    assert_eq!(fs::read(&updated).unwrap(), b"updated-before");
    assert_eq!(fs::read(&removed).unwrap(), b"removed-before");
    assert!(!created.exists());
    assert_eq!(fs::read(&failed).unwrap(), b"failed-before");
    assert_eq!(mode(&updated), 0o640);
    assert_eq!(mode(&removed), 0o604);
    assert_eq!(mode(&failed), 0o640);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn injected_failure_recreates_removed_predecessor_directory_before_its_files() {
    let home = tempfile_path("directory-rollback");
    let predecessor_dir = home.join("smll-proxy");
    let predecessor = predecessor_dir.join("index.ts");
    let plugin = home.join("tapas.js");
    fs::create_dir_all(&predecessor_dir).unwrap();
    fs::write(&predecessor, b"predecessor").unwrap();
    fs::set_permissions(&predecessor_dir, Permissions::from_mode(0o750)).unwrap();
    fs::set_permissions(&predecessor, Permissions::from_mode(0o640)).unwrap();

    let mut transaction = Transaction::new();
    transaction.remove_file(&predecessor).unwrap();
    transaction
        .remove_empty_directory(&predecessor_dir)
        .unwrap();
    transaction
        .write(&plugin, b"plugin".to_vec(), 0o600)
        .unwrap();
    let failure = transaction
        .commit_with(|index| {
            if index == 2 {
                Err(io::Error::other("injected plugin failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(failure.rollback_failures.is_empty());
    assert_eq!(fs::read(&predecessor).unwrap(), b"predecessor");
    assert_eq!(mode(&predecessor), 0o640);
    assert_eq!(mode(&predecessor_dir), 0o750);
    assert!(!plugin.exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn injected_second_delete_failure_restores_opencode_plugin_bytes_and_mode() {
    let home = tempfile_path("opencode-unsetup-rollback");
    fs::create_dir_all(&home).unwrap();
    let plugin = home.join("tapas.js");
    let ownership = home.join("opencode.owned");
    fs::write(&plugin, b"plugin-before").unwrap();
    fs::write(&ownership, b"ownership-before").unwrap();
    fs::set_permissions(&plugin, Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(&ownership, Permissions::from_mode(0o600)).unwrap();

    let mut transaction = Transaction::new();
    transaction.remove_file(&plugin).unwrap();
    transaction.remove_file(&ownership).unwrap();
    let failure = transaction
        .commit_with(|index| {
            if index == 1 {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected ownership deletion failure",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(failure.rollback_failures.is_empty());
    assert_eq!(failure.error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read(&plugin).unwrap(), b"plugin-before");
    assert_eq!(fs::read(&ownership).unwrap(), b"ownership-before");
    assert_eq!(mode(&plugin), 0o640);
    assert_eq!(mode(&ownership), 0o600);
    fs::remove_dir_all(home).unwrap();
}

fn mode(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn tempfile_path(label: &str) -> std::path::PathBuf {
    let sequence = super::TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tapas-transaction-{label}-{}-{sequence}",
        std::process::id()
    ))
}
