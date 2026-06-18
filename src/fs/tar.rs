use std::io::{Write, empty};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dashmap::DashSet;
use tar::{Builder, EntryType, Header};
use tracing::debug;

use crate::fs::Destination;

pub(crate) struct TarDestination<W: Write> {
    builder: Mutex<Builder<W>>,
    seen: DashSet<PathBuf>,
    uid: u64,
    gid: u64,
}

impl<W: Write> TarDestination<W> {
    pub(crate) fn new(writer: W, uid: u64, gid: u64) -> Self {
        Self {
            builder: Mutex::new(Builder::new(writer)),
            seen: DashSet::new(),
            uid,
            gid,
        }
    }

    fn create_dir(&self, path: &Path) -> std::io::Result<()> {
        debug!(path = %path.display(), "Creating directory");

        let mut header = Header::new_gnu();

        header.set_entry_type(EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_uid(self.uid);
        header.set_gid(self.gid);
        header.set_cksum();

        self.builder
            .lock()
            .unwrap()
            .append_data(&mut header, path, empty())
    }

    fn ensure_dir(&self, path: &Path) -> std::io::Result<()> {
        debug!(path = %path.display(), "Ensuring directory");

        let paths = path
            .parent()
            .into_iter()
            .flat_map(Path::ancestors)
            .filter(|path| *path != Path::new(".") && !path.as_os_str().is_empty())
            .collect::<Vec<_>>();

        for path in paths.into_iter().rev() {
            if self.seen.insert(path.to_path_buf()) {
                self.create_dir(path)?;
            }
        }

        Ok(())
    }
}

impl<W: Write + Send> Destination for TarDestination<W> {
    fn write(&self, path: &Path, content: &[u8], mode: u32) -> std::io::Result<()> {
        let path = to_relative_path(path);

        debug!(path = %path.display(), size = content.len(), mode, "Writing file");

        self.ensure_dir(&path)?;

        let mut header = Header::new_gnu();

        header.set_size(content.len() as u64);
        header.set_mode(mode);
        header.set_uid(self.uid);
        header.set_gid(self.gid);
        header.set_cksum();

        self.builder
            .lock()
            .unwrap()
            .append_data(&mut header, path, content)
    }

    fn symlink(&self, path: &Path, target: &Path, mode: u32) -> std::io::Result<()> {
        let path = to_relative_path(path);

        debug!(path = %path.display(), target = %target.display(), mode, "Creating symlink");

        self.ensure_dir(&path)?;

        let mut header = Header::new_gnu();

        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_mode(mode);
        header.set_uid(self.uid);
        header.set_gid(self.gid);
        header.set_cksum();

        self.builder
            .lock()
            .unwrap()
            .append_link(&mut header, path, target)
    }
}

fn to_relative_path(path: &Path) -> PathBuf {
    Path::new("./").join(path.strip_prefix("/").unwrap_or(path))
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[derive(Debug)]
    struct ArchiveEntry {
        path: PathBuf,
        entry_type: EntryType,
        mode: u32,
        uid: u64,
        gid: u64,
        link_name: Option<PathBuf>,
        content: Vec<u8>,
    }

    fn read_archive(buffer: &[u8]) -> Vec<ArchiveEntry> {
        tar::Archive::new(buffer)
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let header = entry.header().clone();
                let path = entry.path().unwrap().into_owned();
                let link_name = entry.link_name().unwrap().map(|path| path.into_owned());
                let mut content = Vec::new();

                entry.read_to_end(&mut content).unwrap();

                ArchiveEntry {
                    path,
                    entry_type: header.entry_type(),
                    mode: header.mode().unwrap(),
                    uid: header.uid().unwrap(),
                    gid: header.gid().unwrap(),
                    link_name,
                    content,
                }
            })
            .collect()
    }

    fn find_entry<'a>(entries: &'a [ArchiveEntry], path: &str) -> &'a ArchiveEntry {
        entries
            .iter()
            .find(|entry| entry.path == Path::new(path))
            .unwrap_or_else(|| panic!("missing archive entry: {path}"))
    }

    #[test]
    fn writes_files_with_parent_directories_and_metadata() {
        let mut buffer = Vec::new();

        {
            let destination = TarDestination::new(&mut buffer, 1000, 1001);

            destination
                .write(Path::new("/usr/bin/ocinoco"), b"binary", 0o755)
                .unwrap();
            destination
                .write(Path::new("/usr/bin/helper"), b"helper", 0o700)
                .unwrap();
        }

        let entries = read_archive(&buffer);
        let usr = find_entry(&entries, "usr");
        let bin = find_entry(&entries, "usr/bin");
        let file = find_entry(&entries, "usr/bin/ocinoco");

        assert_eq!(usr.entry_type, EntryType::Directory);
        assert_eq!(bin.entry_type, EntryType::Directory);
        assert_eq!(file.entry_type, EntryType::Regular);
        assert_eq!(file.content, b"binary");
        assert_eq!(file.mode, 0o755);
        assert_eq!(file.uid, 1000);
        assert_eq!(file.gid, 1001);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.path == Path::new("usr/bin"))
                .count(),
            1
        );
        assert!(!entries.iter().any(|entry| entry.path == Path::new(".")));
    }

    #[test]
    fn writes_symlinks_with_relative_targets() {
        let mut buffer = Vec::new();

        {
            let destination = TarDestination::new(&mut buffer, 123, 456);

            destination
                .symlink(Path::new("/usr/bin/tool"), Path::new("../lib/tool"), 0o777)
                .unwrap();
        }

        let entries = read_archive(&buffer);
        let symlink = find_entry(&entries, "usr/bin/tool");

        assert_eq!(symlink.entry_type, EntryType::Symlink);
        assert_eq!(symlink.link_name.as_deref(), Some(Path::new("../lib/tool")));
        assert_eq!(symlink.mode, 0o777);
        assert_eq!(symlink.uid, 123);
        assert_eq!(symlink.gid, 456);
    }

    #[test]
    fn normalizes_relative_and_absolute_paths() {
        assert_eq!(
            to_relative_path(Path::new("/etc/config")),
            Path::new("./etc/config")
        );
        assert_eq!(
            to_relative_path(Path::new("etc/config")),
            Path::new("./etc/config")
        );
    }
}
