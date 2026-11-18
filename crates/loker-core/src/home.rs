use std::path::{Path, PathBuf};

/// `LOKER_HOME` if set, otherwise `~/.loker`.
pub fn loker_home() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("LOKER_HOME") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow::anyhow!("HOME is not set; set LOKER_HOME instead"))?;
    Ok(PathBuf::from(home).join(".loker"))
}

pub fn db_path(home: &Path) -> PathBuf {
    home.join("loker.db")
}
