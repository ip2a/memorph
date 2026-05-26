use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

pub fn write_string_atomic(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Path has no parent directory: {}", path.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temporary file in {}", parent.display()))?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("Failed to write temporary file for {}", path.display()))?;
    file.as_file()
        .sync_all()
        .with_context(|| format!("Failed to sync temporary file for {}", path.display()))?;
    file.persist(path)
        .map_err(|err| err.error)
        .with_context(|| format!("Failed to persist temporary file to {}", path.display()))?;
    Ok(())
}
