use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::MAX_CONFIG_BYTES;
use super::storage::{read_optional, write_atomic};

pub(super) struct Transaction {
    mutations: Vec<Mutation>,
}

pub(super) struct Failure {
    pub error: io::Error,
    pub rollback_failures: Vec<RollbackFailure>,
}

pub(super) struct RollbackFailure {
    pub path: PathBuf,
    pub error: io::Error,
}

impl std::fmt::Display for RollbackFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.error)
    }
}

enum MutationKind {
    Write { content: Vec<u8>, mode: u32 },
    RemoveFile,
    RemoveEmptyDirectory,
}

struct Mutation {
    path: PathBuf,
    before: FileState,
    kind: MutationKind,
}

#[derive(Eq, PartialEq)]
enum FileState {
    Missing,
    File { content: Vec<u8>, mode: u32 },
    Directory { mode: u32 },
}

impl Transaction {
    pub fn new() -> Self {
        Self {
            mutations: Vec::new(),
        }
    }

    pub fn write(&mut self, path: &Path, content: Vec<u8>, mode: u32) -> io::Result<()> {
        self.push(path, MutationKind::Write { content, mode })
    }

    pub fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        self.push(path, MutationKind::RemoveFile)
    }

    pub fn remove_empty_directory(&mut self, path: &Path) -> io::Result<()> {
        self.push(path, MutationKind::RemoveEmptyDirectory)
    }

    fn push(&mut self, path: &Path, kind: MutationKind) -> io::Result<()> {
        if self.mutations.iter().any(|mutation| mutation.path == path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("duplicate transaction path: {}", path.display()),
            ));
        }
        self.mutations.push(Mutation {
            path: path.to_path_buf(),
            before: FileState::read(path)?,
            kind,
        });
        Ok(())
    }

    pub fn commit(self) -> Result<(), Failure> {
        self.commit_with(|_| Ok(()))
    }

    pub(super) fn commit_with(
        self,
        mut before_apply: impl FnMut(usize) -> io::Result<()>,
    ) -> Result<(), Failure> {
        self.commit_inner(&mut before_apply, &mut |_| Ok(()))
    }

    #[cfg(test)]
    pub(super) fn commit_with_after_apply(
        self,
        mut after_apply: impl FnMut(usize) -> io::Result<()>,
    ) -> Result<(), Failure> {
        self.commit_inner(&mut |_| Ok(()), &mut after_apply)
    }

    fn commit_inner(
        self,
        before_apply: &mut impl FnMut(usize) -> io::Result<()>,
        after_apply: &mut impl FnMut(usize) -> io::Result<()>,
    ) -> Result<(), Failure> {
        let mut applied = Vec::new();
        for (index, mutation) in self.mutations.iter().enumerate() {
            let ready = before_apply(index).and_then(|()| mutation.verify());
            let operation_started = ready.is_ok();
            let result = ready
                .and_then(|()| mutation.apply())
                .and_then(|changed| after_apply(index).map(|()| changed));
            match result {
                Ok(changed) => applied.push((mutation, changed)),
                Err(error) => {
                    let mut rollback_failures = Vec::new();
                    if operation_started
                        && mutation.write_may_have_changed_path()
                        && let Err(rollback) = mutation.before.restore(&mutation.path)
                    {
                        rollback_failures.push(RollbackFailure {
                            path: mutation.path.clone(),
                            error: rollback,
                        });
                    }
                    for (applied, changed) in applied.into_iter().rev() {
                        if changed && let Err(rollback) = applied.before.restore(&applied.path) {
                            rollback_failures.push(RollbackFailure {
                                path: applied.path.clone(),
                                error: rollback,
                            });
                        }
                    }
                    return Err(Failure {
                        error,
                        rollback_failures,
                    });
                }
            }
        }
        Ok(())
    }
}

impl Mutation {
    fn verify(&self) -> io::Result<()> {
        if matches!(self.kind, MutationKind::RemoveEmptyDirectory) {
            Ok(())
        } else {
            self.before.verify(&self.path)
        }
    }

    fn apply(&self) -> io::Result<bool> {
        match &self.kind {
            MutationKind::Write { content, mode } => {
                write_atomic(&self.path, content, *mode)?;
                Ok(true)
            }
            MutationKind::RemoveFile => {
                fs::remove_file(&self.path)?;
                Ok(true)
            }
            MutationKind::RemoveEmptyDirectory => match fs::remove_dir(&self.path) {
                Ok(()) => Ok(true),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                    ) =>
                {
                    Ok(false)
                }
                Err(error) => Err(error),
            },
        }
    }

    fn write_may_have_changed_path(&self) -> bool {
        matches!(self.kind, MutationKind::Write { .. })
    }
}

impl FileState {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(error) => return Err(error),
        };
        let mode = metadata.permissions().mode() & 0o777;
        if metadata.is_file() {
            let content = read_optional(path, MAX_CONFIG_BYTES)?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "transaction input disappeared")
            })?;
            Ok(Self::File { content, mode })
        } else if metadata.is_dir() {
            Ok(Self::Directory { mode })
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("transaction path is not a regular file: {}", path.display()),
            ))
        }
    }

    fn verify(&self, path: &Path) -> io::Result<()> {
        let current = Self::read(path)?;
        if self == &current {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "file changed during setup: {}",
                path.display()
            )))
        }
    }

    fn restore(&self, path: &Path) -> io::Result<()> {
        match self {
            Self::Missing => match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
            Self::File { content, mode } => write_atomic(path, content, *mode),
            Self::Directory { mode } => {
                fs::create_dir(path)?;
                fs::set_permissions(path, Permissions::from_mode(*mode))
            }
        }
    }
}
