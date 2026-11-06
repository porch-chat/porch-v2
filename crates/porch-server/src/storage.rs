//! Blocking object storage. Call only behind a bounded spawn_blocking permit.
//! The private operator-owned root is the filesystem security boundary.
use porch_protocol::{MAX_UPLOAD_BYTES, UPLOAD_CHUNK_BYTES};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub trait Storage: Send + Sync {
    /// Caller serializes each key; offset is the last SQL-acknowledged position.
    fn put_chunk(&self, key: &str, offset: u64, bytes: &[u8]) -> io::Result<()>;
    fn chunk_matches(&self, key: &str, offset: u64, bytes: &[u8]) -> io::Result<bool>;
    fn finalize(&self, key: &str, bytes: u64, sha256: &str) -> io::Result<String>;
    fn open(&self, key: &str) -> io::Result<File>;
    fn remove(&self, key: &str) -> io::Result<()>;
}
#[derive(Clone)]
pub struct LocalStorage {
    root: PathBuf,
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid object operation")
}
pub fn valid_key(key: &str) -> bool {
    key.len() == 32
        && key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
impl LocalStorage {
    /// Existing roots must already be private, never silently chmod user data.
    pub fn new(root: &Path) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(invalid());
        }
        if !root.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().mode(0o700).create(root)?;
            }
            #[cfg(not(unix))]
            fs::create_dir(root)?;
        }
        let meta = fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(invalid());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "storage root must be private",
                ));
            }
        }
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    fn path(&self, key: &str, suffix: &str) -> io::Result<PathBuf> {
        if !valid_key(key) {
            return Err(invalid());
        }
        Ok(self.root.join(format!("{key}-{suffix}")))
    }
    fn chunk_path(&self, key: &str, offset: u64) -> io::Result<PathBuf> {
        if offset >= MAX_UPLOAD_BYTES || !offset.is_multiple_of(UPLOAD_CHUNK_BYTES) {
            return Err(invalid());
        }
        self.path(key, &format!("{offset:010}.chunk"))
    }
    fn checked_open(path: &Path) -> io::Result<File> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(invalid());
        }
        // Root is private. An attacker with this process UID can already read all
        // credentials. No safety claim against an adversarial same-UID operator.
        File::open(path)
    }
    fn sync_directory(&self) -> io::Result<()> {
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}
impl Storage for LocalStorage {
    fn put_chunk(&self, key: &str, offset: u64, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty()
            || bytes.len() as u64 > UPLOAD_CHUNK_BYTES
            || offset
                .checked_add(bytes.len() as u64)
                .is_none_or(|end| end > MAX_UPLOAD_BYTES)
        {
            return Err(invalid());
        }
        let path = self.chunk_path(key, offset)?;
        let mut tmp = tempfile::Builder::new()
            .prefix(&format!("{key}-tmp-"))
            .tempfile_in(&self.root)?;
        tmp.write_all(bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| e.error)?;
        self.sync_directory()
    }
    fn chunk_matches(&self, key: &str, offset: u64, bytes: &[u8]) -> io::Result<bool> {
        if bytes.is_empty() || bytes.len() as u64 > UPLOAD_CHUNK_BYTES {
            return Err(invalid());
        }
        let file = Self::checked_open(&self.chunk_path(key, offset)?)?;
        if file.metadata()?.len() != bytes.len() as u64 {
            return Ok(false);
        }
        let mut stored = Vec::with_capacity(bytes.len());
        file.take(UPLOAD_CHUNK_BYTES + 1).read_to_end(&mut stored)?;
        Ok(stored == bytes)
    }
    fn finalize(&self, key: &str, bytes: u64, sha256: &str) -> io::Result<String> {
        if bytes == 0
            || bytes > MAX_UPLOAD_BYTES
            || sha256.len() != 64
            || !sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid());
        }
        let object = self.path(key, "original")?;
        let mut tmp = tempfile::Builder::new()
            .prefix(&format!("{key}-tmp-"))
            .tempfile_in(&self.root)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut prefix = Vec::with_capacity(8192);
        let mut offset = 0;
        while offset < bytes {
            let mut chunk = Self::checked_open(&self.chunk_path(key, offset)?)?;
            let expected = (bytes - offset).min(UPLOAD_CHUNK_BYTES);
            if chunk.metadata()?.len() != expected {
                return Err(invalid());
            }
            let mut copied = 0;
            loop {
                let n = chunk.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                if prefix.len() < 8192 {
                    prefix.extend_from_slice(&buffer[..n.min(8192 - prefix.len())]);
                }
                hash.update(&buffer[..n]);
                tmp.write_all(&buffer[..n])?;
                copied += n as u64;
            }
            if copied != expected {
                return Err(invalid());
            }
            offset += expected;
        }
        if format!("{:x}", hash.finalize()) != sha256 {
            return Err(invalid());
        }
        tmp.as_file().sync_all()?;
        tmp.persist(object).map_err(|e| e.error)?;
        self.sync_directory()?;
        Ok(infer::get(&prefix)
            .map(|k| k.mime_type().to_string())
            .unwrap_or_else(|| "application/octet-stream".into()))
    }
    fn open(&self, key: &str) -> io::Result<File> {
        Self::checked_open(&self.path(key, "original")?)
    }
    fn remove(&self, key: &str) -> io::Result<()> {
        if !valid_key(key) {
            return Err(invalid());
        }
        let prefix = format!("{key}-");
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                // unlink only, never follow a symlink or recurse into anything.
                fs::remove_file(entry.path())?;
            }
        }
        self.sync_directory()
    }
}

/// Inclusive single byte range. Suffix ranges supported; multi-range rejected.
pub fn byte_range(value: Option<&str>, bytes: u64) -> io::Result<(u64, u64)> {
    if bytes == 0 {
        return Err(invalid());
    }
    let Some(value) = value else {
        return Ok((0, bytes - 1));
    };
    let value = value.strip_prefix("bytes=").ok_or_else(invalid)?;
    let (a, b) = value.split_once('-').ok_or_else(invalid)?;
    fn number(s: &str) -> io::Result<u64> {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        s.parse().map_err(|_| invalid())
    }
    if a.is_empty() {
        let suffix = number(b)?;
        if suffix == 0 {
            return Err(invalid());
        }
        return Ok((bytes.saturating_sub(suffix), bytes - 1));
    }
    let start = number(a)?;
    let end = if b.is_empty() {
        bytes - 1
    } else {
        number(b)?.min(bytes - 1)
    };
    if start >= bytes || end < start {
        return Err(invalid());
    }
    Ok((start, end))
}
pub fn seek_range(mut file: File, start: u64) -> io::Result<File> {
    file.seek(SeekFrom::Start(start))?;
    Ok(file)
}
#[cfg(test)]
mod tests {
    use super::*;
    const KEY: &str = "0123456789abcdef0123456789abcdef";
    fn fixture() -> (tempfile::TempDir, LocalStorage) {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalStorage::new(&temp.path().join("private")).unwrap();
        (temp, store)
    }
    #[test]
    fn atomic_retry_hash_and_stream() {
        let (_temp, store) = fixture();
        let bytes = vec![17; UPLOAD_CHUNK_BYTES as usize];
        store.put_chunk(KEY, 0, &bytes).unwrap();
        assert!(store.chunk_matches(KEY, 0, &bytes).unwrap());
        assert!(!store.chunk_matches(KEY, 0, b"bad retry").unwrap());
        // A crash before SQL ack is retried by replacing the same orphan chunk.
        store.put_chunk(KEY, 0, &bytes).unwrap();
        store.put_chunk(KEY, UPLOAD_CHUNK_BYTES, b"tail").unwrap();
        let mut hash = Sha256::new();
        hash.update(&bytes);
        hash.update(b"tail");
        let hash = format!("{:x}", hash.finalize());
        assert!(
            store
                .finalize(KEY, UPLOAD_CHUNK_BYTES + 4, &"0".repeat(64))
                .is_err()
        );
        assert!(store.open(KEY).is_err());
        assert_eq!(
            store.finalize(KEY, UPLOAD_CHUNK_BYTES + 4, &hash).unwrap(),
            "application/octet-stream"
        );
        assert_eq!(
            store.open(KEY).unwrap().metadata().unwrap().len(),
            UPLOAD_CHUNK_BYTES + 4
        );
        assert!(store.finalize(KEY, UPLOAD_CHUNK_BYTES + 4, &hash).is_ok());
        store.remove(KEY).unwrap();
        assert!(store.open(KEY).is_err());
    }
    #[test]
    fn rejects_path_injection_and_bounds() {
        let (_temp, store) = fixture();
        for key in [
            "../secret",
            "",
            "/etc/passwd",
            "0123456789abcdef0123456789abcdeF",
            "x\r\ny",
        ] {
            assert!(store.put_chunk(key, 0, b"x").is_err());
            assert!(store.open(key).is_err());
            assert!(store.remove(key).is_err());
        }
        assert!(store.put_chunk(KEY, 1, b"x").is_err());
        assert!(store.put_chunk(KEY, MAX_UPLOAD_BYTES, b"x").is_err());
        assert!(store.put_chunk(KEY, 0, b"").is_err());
        assert!(
            store
                .finalize(KEY, MAX_UPLOAD_BYTES + 1, &"0".repeat(64))
                .is_err()
        );
    }
    #[test]
    fn single_ranges() {
        for (value, result) in [
            (None, (0, 99)),
            (Some("bytes=10-19"), (10, 19)),
            (Some("bytes=90-"), (90, 99)),
            (Some("bytes=-20"), (80, 99)),
            (Some("bytes=-200"), (0, 99)),
            (Some("bytes=0-200"), (0, 99)),
        ] {
            assert_eq!(byte_range(value, 100).unwrap(), result);
        }
        for bad in [
            "bytes=-0",
            "bytes=100-",
            "bytes=50-40",
            "bytes=0-1,3-4",
            "bytes=+1-4",
            "bytes=1 -2",
            "x=0-1",
            "bytes=0-1-2",
        ] {
            assert!(byte_range(Some(bad), 100).is_err(), "{bad}");
        }
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_public_root() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (temp, store) = fixture();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"secret").unwrap();
        symlink(&outside, store.path(KEY, "original").unwrap()).unwrap();
        assert!(store.open(KEY).is_err());
        store.remove(KEY).unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"secret");
        let root = temp.path().join("public");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(LocalStorage::new(&root).is_err());
        let link = temp.path().join("link");
        symlink(&root, &link).unwrap();
        assert!(LocalStorage::new(&link).is_err());
    }
    #[test]
    #[ignore = "writes 1 GiB into disposable local storage; CI runs serially"]
    fn full_gibibyte_stream() {
        let (_temp, store) = fixture();
        let chunk = vec![0u8; UPLOAD_CHUNK_BYTES as usize];
        let mut hash = Sha256::new();
        for offset in (0..MAX_UPLOAD_BYTES).step_by(UPLOAD_CHUNK_BYTES as usize) {
            store.put_chunk(KEY, offset, &chunk).unwrap();
            hash.update(&chunk);
        }
        store
            .finalize(KEY, MAX_UPLOAD_BYTES, &format!("{:x}", hash.finalize()))
            .unwrap();
        assert_eq!(
            store.open(KEY).unwrap().metadata().unwrap().len(),
            MAX_UPLOAD_BYTES
        );
        store.remove(KEY).unwrap();
    }
}
