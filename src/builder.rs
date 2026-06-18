use std::io::{Error, Result};
use std::path::{Path, PathBuf};

use pathdiff::diff_paths;
use rayon::iter::ParallelIterator;
use tracing::debug;

use crate::fs::{Destination, DirEntry, Source};

/// Builder copies the files from the source to the destination.
pub(crate) struct Builder<S: Source, D: Destination> {
    source: S,
    destination: D,
    root_dir: PathBuf,
}

impl<S: Source, D: Destination> Builder<S, D> {
    pub(crate) fn new(source: S, destination: D, root_dir: impl Into<PathBuf>) -> Self {
        Self {
            source,
            destination,
            root_dir: root_dir.into(),
        }
    }

    pub(crate) fn build(&self) -> Result<()> {
        self.copy_dir(Path::new("/"))
    }

    fn copy_dir(&self, path: &Path) -> Result<()> {
        debug!(path = %path.display(), "Copying directory");

        self.source
            .read_dir(path)?
            .map(|entry| match entry {
                DirEntry::Directory(path) => self.copy_dir(&path),
                DirEntry::File(file) => {
                    let content = self.source.read(&file.path)?;

                    self.destination
                        .write(&self.destination_path(&file.path), &content, file.mode)
                }
                DirEntry::Symlink(symlink) => {
                    let target = if symlink.target.is_absolute() {
                        let parent = symlink
                            .path
                            .parent()
                            .ok_or_else(|| Error::other("Invalid symlink path"))?;

                        diff_paths(&symlink.target, parent)
                            .ok_or_else(|| Error::other("Invalid symlink target"))?
                    } else {
                        symlink.target
                    };

                    self.destination.symlink(
                        &self.destination_path(&symlink.path),
                        &target,
                        symlink.mode,
                    )
                }
            })
            .collect()
    }

    fn destination_path(&self, path: &Path) -> PathBuf {
        self.root_dir.join(path.strip_prefix("/").unwrap_or(path))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use rayon::iter::{IntoParallelIterator, ParallelIterator};

    use super::*;
    use crate::fs::{File, Symlink};

    #[derive(Default)]
    struct FakeSource {
        dirs: HashMap<PathBuf, Vec<DirEntry>>,
        files: HashMap<PathBuf, Vec<u8>>,
    }

    impl Source for FakeSource {
        fn read_dir(&self, path: &Path) -> Result<impl ParallelIterator<Item = DirEntry>> {
            Ok(self
                .dirs
                .get(path)
                .cloned()
                .unwrap_or_default()
                .into_par_iter())
        }

        fn read(&self, path: &Path) -> Result<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "missing file"))
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Event {
        Write(PathBuf, Vec<u8>, u32),
        Symlink(PathBuf, PathBuf, u32),
    }

    #[derive(Clone, Default)]
    struct RecordingDestination {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl Destination for RecordingDestination {
        fn write(&self, path: &Path, content: &[u8], mode: u32) -> Result<()> {
            self.events.lock().unwrap().push(Event::Write(
                path.to_path_buf(),
                content.to_vec(),
                mode,
            ));

            Ok(())
        }

        fn symlink(&self, path: &Path, target: &Path, mode: u32) -> Result<()> {
            self.events.lock().unwrap().push(Event::Symlink(
                path.to_path_buf(),
                target.to_path_buf(),
                mode,
            ));

            Ok(())
        }
    }

    #[test]
    fn recursively_copies_files() {
        let mut source = FakeSource::default();
        source.dirs.insert(
            PathBuf::from("/"),
            vec![DirEntry::Directory(PathBuf::from("/app"))],
        );
        source.dirs.insert(
            PathBuf::from("/app"),
            vec![DirEntry::File(File {
                path: PathBuf::from("/app/main"),
                mode: 0o755,
            })],
        );
        source
            .files
            .insert(PathBuf::from("/app/main"), b"binary".to_vec());

        let destination = RecordingDestination::default();

        Builder::new(source, destination.clone(), "/")
            .build()
            .unwrap();

        assert_eq!(
            destination.events.lock().unwrap().as_slice(),
            [Event::Write(
                PathBuf::from("/app/main"),
                b"binary".to_vec(),
                0o755
            )]
        );
    }

    #[test]
    fn makes_absolute_symlink_targets_relative_to_the_symlink_parent() {
        let mut source = FakeSource::default();
        source.dirs.insert(
            PathBuf::from("/"),
            vec![DirEntry::Symlink(Symlink {
                path: PathBuf::from("/app/current"),
                target: PathBuf::from("/app/releases/v1"),
                mode: 0o777,
            })],
        );

        let destination = RecordingDestination::default();

        Builder::new(source, destination.clone(), "/")
            .build()
            .unwrap();

        assert_eq!(
            destination.events.lock().unwrap().as_slice(),
            [Event::Symlink(
                PathBuf::from("/app/current"),
                PathBuf::from("releases/v1"),
                0o777
            )]
        );
    }

    #[test]
    fn preserves_relative_symlink_targets() {
        let mut source = FakeSource::default();
        source.dirs.insert(
            PathBuf::from("/"),
            vec![DirEntry::Symlink(Symlink {
                path: PathBuf::from("/app/current"),
                target: PathBuf::from("releases/v1"),
                mode: 0o777,
            })],
        );

        let destination = RecordingDestination::default();

        Builder::new(source, destination.clone(), "/")
            .build()
            .unwrap();

        assert_eq!(
            destination.events.lock().unwrap().as_slice(),
            [Event::Symlink(
                PathBuf::from("/app/current"),
                PathBuf::from("releases/v1"),
                0o777
            )]
        );
    }

    #[test]
    fn prefixes_destination_paths_with_root_dir() {
        let mut source = FakeSource::default();
        source.dirs.insert(
            PathBuf::from("/"),
            vec![
                DirEntry::File(File {
                    path: PathBuf::from("/bin/tool"),
                    mode: 0o755,
                }),
                DirEntry::Symlink(Symlink {
                    path: PathBuf::from("/bin/current"),
                    target: PathBuf::from("tool"),
                    mode: 0o777,
                }),
            ],
        );
        source
            .files
            .insert(PathBuf::from("/bin/tool"), b"tool".to_vec());

        let destination = RecordingDestination::default();

        Builder::new(source, destination.clone(), "/app")
            .build()
            .unwrap();

        let events = destination.events.lock().unwrap();

        assert_eq!(events.len(), 2);
        assert!(events.contains(&Event::Write(
            PathBuf::from("/app/bin/tool"),
            b"tool".to_vec(),
            0o755
        )));
        assert!(events.contains(&Event::Symlink(
            PathBuf::from("/app/bin/current"),
            PathBuf::from("tool"),
            0o777
        )));
    }
}
