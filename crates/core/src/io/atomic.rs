use crate::error::Result;
use std::io::Write;
use std::path::Path;

/// Write `bytes` to `path` atomically — data is flushed and the temporary
/// file is renamed into place, so a partial write is never visible.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file_mut().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
