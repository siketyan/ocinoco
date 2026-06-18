use std::io::Write;

use sha2::Digest;
use sha2::digest::array::Array;

pub(crate) struct HashedWriter<D: Digest, W: Write> {
    hasher: D,
    writer: W,
}

impl<D: Digest, W: Write> HashedWriter<D, W> {
    pub(crate) fn new(hasher: D, writer: W) -> Self {
        Self { hasher, writer }
    }

    pub(crate) fn finalize(self) -> Array<u8, D::OutputSize> {
        self.hasher.finalize()
    }
}

impl<D: Digest, W: Write> Write for HashedWriter<D, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.writer.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.hasher.update(buf);
        self.writer.write_all(buf)
    }
}

#[cfg(test)]
mod tests {
    use sha2::Sha256;

    use super::*;

    #[test]
    fn hashes_and_forwards_written_bytes() {
        let mut output = Vec::new();
        let digest = {
            let mut writer = HashedWriter::new(Sha256::new(), &mut output);

            writer.write_all(b"hello").unwrap();
            writer.write_all(b" world").unwrap();
            writer.flush().unwrap();
            writer.finalize()
        };

        assert_eq!(output, b"hello world");
        assert_eq!(digest.as_slice(), Sha256::digest(b"hello world").as_slice());
    }
}
