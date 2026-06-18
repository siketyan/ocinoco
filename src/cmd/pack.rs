use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use tracing::info;

use crate::builder::Builder;
use crate::fs::os::OsSource;
use crate::fs::tar::TarDestination;

/// Pack files into a tar.zst archive.
#[derive(Clone, Debug, clap::Parser)]
pub(crate) struct Args {
    /// Path to the source directory.
    source: PathBuf,

    /// Path to the output tar.zst archive.
    output: PathBuf,

    /// Directory in the archive to copy the files into (defaults to /).
    #[clap(short, long)]
    root_dir: Option<String>,

    /// Change the owner UID of files copied to the archive (defaults to 0).
    #[clap(long)]
    uid: Option<u64>,

    /// Change the owner GID of files copied to the archive (defaults to 0).
    #[clap(long)]
    gid: Option<u64>,
}

pub(super) async fn run(args: Args) -> anyhow::Result<()> {
    info!(
        source = %args.source.display(),
        output = %args.output.display(),
        "Packing archive"
    );

    let source = OsSource::new(args.source);
    let file = File::create(args.output)?;
    let writer = BufWriter::new(file);
    let mut encoder = zstd::Encoder::new(writer, 0)?;

    {
        let destination =
            TarDestination::new(&mut encoder, args.uid.unwrap_or(0), args.gid.unwrap_or(0));

        Builder::new(
            source,
            destination,
            args.root_dir.unwrap_or_else(|| "/".to_string()),
        )
        .build()?;
    }

    let mut writer = encoder.finish()?;
    writer.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use tar::EntryType;

    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("ocinoco-pack-test-{}-{nanos}", std::process::id()));

            fs::create_dir_all(&path).unwrap();

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

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

    fn read_archive(path: &Path) -> Vec<ArchiveEntry> {
        let file = fs::File::open(path).unwrap();
        let decoder = zstd::Decoder::new(file).unwrap();

        tar::Archive::new(decoder)
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

    #[tokio::test]
    async fn packs_real_filesystem_into_valid_tar_zst_archive() {
        let temp = TempDir::new();
        let source = temp.path().join("source");
        let output = temp.path().join("layer.tzst");

        fs::create_dir_all(source.join("bin")).unwrap();
        fs::create_dir_all(source.join("share/doc")).unwrap();
        fs::write(source.join("bin/tool"), b"#!/bin/sh\necho tool\n").unwrap();
        fs::write(source.join("share/doc/readme.txt"), b"hello from ocinoco\n").unwrap();

        #[cfg(unix)]
        {
            fs::set_permissions(source.join("bin/tool"), fs::Permissions::from_mode(0o755))
                .unwrap();
            fs::set_permissions(
                source.join("share/doc/readme.txt"),
                fs::Permissions::from_mode(0o640),
            )
            .unwrap();
            symlink("tool", source.join("bin/current")).unwrap();
        }

        run(Args {
            source,
            output: output.clone(),
            root_dir: Some("/app".to_string()),
            uid: Some(1000),
            gid: Some(1001),
        })
        .await
        .unwrap();

        let entries = read_archive(&output);
        let app = find_entry(&entries, "app");
        let bin = find_entry(&entries, "app/bin");
        let tool = find_entry(&entries, "app/bin/tool");
        let readme = find_entry(&entries, "app/share/doc/readme.txt");

        assert_eq!(app.entry_type, EntryType::Directory);
        assert_eq!(bin.entry_type, EntryType::Directory);
        assert_eq!(tool.entry_type, EntryType::Regular);
        assert_eq!(tool.content, b"#!/bin/sh\necho tool\n");
        assert_eq!(tool.uid, 1000);
        assert_eq!(tool.gid, 1001);
        assert_eq!(tool.mode & 0o777, 0o755);
        assert_eq!(readme.entry_type, EntryType::Regular);
        assert_eq!(readme.content, b"hello from ocinoco\n");
        assert_eq!(readme.uid, 1000);
        assert_eq!(readme.gid, 1001);

        #[cfg(unix)]
        {
            let symlink = find_entry(&entries, "app/bin/current");

            assert_eq!(symlink.entry_type, EntryType::Symlink);
            assert_eq!(symlink.link_name.as_deref(), Some(Path::new("tool")));
            assert_eq!(symlink.uid, 1000);
            assert_eq!(symlink.gid, 1001);
        }
    }
}
