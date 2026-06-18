use std::io::Result;
use std::path::{Path, PathBuf};

use rayon::iter::{ParallelBridge, ParallelIterator};

use crate::fs::{DirEntry, File, Source, Symlink};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub(crate) struct OsSource {
    root_dir: PathBuf,
}

impl OsSource {
    pub(crate) fn new(root_dir: impl AsRef<Path>) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
        }
    }
}

impl Source for OsSource {
    fn read_dir(&self, path: &Path) -> Result<impl ParallelIterator<Item = DirEntry>> {
        let root_dir = self.root_dir.clone();

        Ok(std::fs::read_dir(self.resolve(path))?
            .par_bridge()
            .filter_map(move |entry| {
                let entry = entry.ok()?;
                let file_type = entry.file_type().ok()?;
                let path = entry.path().strip_prefix(&root_dir).ok()?.to_path_buf();

                if file_type.is_symlink() {
                    Some(DirEntry::Symlink(Symlink {
                        path,
                        target: entry.path().read_link().ok()?,
                        #[cfg(unix)]
                        mode: entry.metadata().ok()?.mode(),
                        #[cfg(not(unix))]
                        mode: 644,
                    }))
                } else if file_type.is_file() {
                    Some(DirEntry::File(File {
                        path,
                        #[cfg(unix)]
                        mode: entry.metadata().ok()?.mode(),
                        #[cfg(not(unix))]
                        mode: 644,
                    }))
                } else if file_type.is_dir() {
                    Some(DirEntry::Directory(path))
                } else {
                    None
                }
            }))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        std::fs::read(self.resolve(path))
    }
}

impl OsSource {
    fn resolve(&self, path: &Path) -> PathBuf {
        self.root_dir.join(path.strip_prefix("/").unwrap_or(path))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rayon::iter::ParallelIterator;

    use super::*;

    fn temp_root() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        std::env::temp_dir().join(format!("ocinoco-os-source-{nanos}"))
    }

    #[test]
    fn reads_paths_under_root_dir_even_when_path_is_absolute() {
        let root = temp_root();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/file.txt"), b"content").unwrap();
        fs::write(root.join("root.txt"), b"root").unwrap();

        let source = OsSource::new(&root);
        let mut entries = source.read_dir(Path::new("/")).unwrap().collect::<Vec<_>>();

        entries.sort_by_key(|entry| match entry {
            DirEntry::Directory(path) => path.clone(),
            DirEntry::File(file) => file.path.clone(),
            DirEntry::Symlink(symlink) => symlink.path.clone(),
        });

        assert!(entries.iter().any(|entry| {
            matches!(entry, DirEntry::Directory(path) if path == Path::new("nested"))
        }));
        assert!(entries.iter().any(|entry| {
            matches!(entry, DirEntry::File(file) if file.path == Path::new("root.txt"))
        }));
        assert_eq!(
            source.read(Path::new("/nested/file.txt")).unwrap(),
            b"content"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_relative_paths_under_root_dir() {
        let root = temp_root();
        let source = OsSource::new(&root);

        assert_eq!(source.resolve(Path::new("/a/b")), root.join("a/b"));
        assert_eq!(source.resolve(Path::new("a/b")), root.join("a/b"));
    }
}
