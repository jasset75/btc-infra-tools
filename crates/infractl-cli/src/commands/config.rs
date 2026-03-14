use anyhow::{Context, Result, bail};
use infractl_core::config::default_config_template;
use std::fs;
use std::path::PathBuf;

pub(crate) fn init_config_file(path: &PathBuf, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "config file already exists at {} (use --force to overwrite)",
            path.display()
        );
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::write(path, default_config_template())
        .with_context(|| format!("failed to write config file {}", path.display()))?;
    Ok(())
}
