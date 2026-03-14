use anyhow::{Context, Result};
use std::path::PathBuf;

/// Loads `.env` values before command execution.
///
/// This is injected so tests can avoid mutating process environment.
pub(crate) trait DotenvLoader {
    fn load_if_present(&self) -> Result<()>;
}

/// Production `.env` loader backed by `dotenvy`.
pub(crate) struct ProcessDotenvLoader;

impl DotenvLoader for ProcessDotenvLoader {
    fn load_if_present(&self) -> Result<()> {
        let path = PathBuf::from(".env");
        if !path.exists() {
            return Ok(());
        }

        dotenvy::from_filename(&path)
            .with_context(|| format!("failed to load environment from {}", path.display()))?;
        Ok(())
    }
}

pub(crate) struct RuntimeDeps<C, E, D> {
    pub(crate) clock: C,
    pub(crate) env_resolver: E,
    /// Strategy used to load dotenv values for the current runtime.
    pub(crate) dotenv_loader: D,
}

#[cfg(test)]
pub(crate) struct NoopDotenvLoader;

#[cfg(test)]
impl DotenvLoader for NoopDotenvLoader {
    fn load_if_present(&self) -> Result<()> {
        Ok(())
    }
}
