pub(crate) mod os;
pub(crate) mod tar;

use std::io::Result;
use std::path::{Path, PathBuf};

use rayon::iter::ParallelIterator;

#[derive(Clone, Debug)]
pub(crate) struct File {
    pub path: PathBuf,
    pub mode: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Symlink {
    pub path: PathBuf,
    pub target: PathBuf,
    pub mode: u32,
}

#[derive(Clone, Debug)]
pub(crate) enum DirEntry {
    Directory(PathBuf),
    File(File),
    Symlink(Symlink),
}

/// Abstracted filesystem for the input.
pub(crate) trait Source: Sync {
    /// Create an iterator to list all files in the directory.
    fn read_dir(&self, path: &Path) -> Result<impl ParallelIterator<Item = DirEntry>>;

    /// Read a file at the given `path`.
    fn read(&self, path: &Path) -> Result<Vec<u8>>;
}

/// Abstracted filesystem for the output.
pub(crate) trait Destination: Sync {
    /// Write the given content to a `path` as a file within the filesystem.
    /// Ancestor directories are created automatically.
    fn write(&self, path: &Path, content: &[u8], mode: u32) -> Result<()>;

    /// Creates a symbolic link at the specified `path` that points to the given `target`.
    /// The target must be a relative path in the same filesystem.
    fn symlink(&self, path: &Path, target: &Path, mode: u32) -> Result<()>;
}
