use anyhow::{Context, Result};
use infractl_core::config::DEFAULT_CONFIG_FILE;
use infractl_core::env::EnvResolver;
use std::path::Path;
use std::path::PathBuf;

const BELTER_CONFIG_ENV: &str = "BELTER_CONFIG";
const BELTER_ENV_FILE_ENV: &str = "BELTER_ENV_FILE";
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
const HOME_ENV: &str = "HOME";

/// Loads `.env` values before command execution.
///
/// This is injected so tests can avoid mutating process environment.
pub(crate) trait DotenvLoader {
    fn load_if_present(&self, config_path: &std::path::Path) -> Result<()>;
}

/// Production `.env` loader backed by `dotenvy`.
pub(crate) struct ProcessDotenvLoader;

impl DotenvLoader for ProcessDotenvLoader {
    fn load_if_present(&self, config_path: &std::path::Path) -> Result<()> {
        let mut candidates = Vec::new();
        if let Some(path) = dotenv_override_path() {
            candidates.push(path);
        }
        if let Some(parent) = config_path.parent() {
            candidates.push(parent.join(".env"));
        }
        candidates.push(PathBuf::from(".env"));

        for path in candidates {
            if !path.exists() {
                continue;
            }

            dotenvy::from_filename(&path)
                .with_context(|| format!("failed to load environment from {}", path.display()))?;
            return Ok(());
        }

        Ok(())
    }
}

pub(crate) fn resolve_config_path(
    explicit_config: Option<&Path>,
    env_resolver: &dyn EnvResolver,
    cwd: &Path,
) -> PathBuf {
    if let Some(path) = explicit_config {
        return path.to_path_buf();
    }

    if let Some(path) = env_resolver.resolve(BELTER_CONFIG_ENV) {
        return PathBuf::from(path);
    }

    if let Some(path) = standard_config_path(env_resolver)
        && path.exists()
    {
        return path;
    }

    cwd.join(DEFAULT_CONFIG_FILE)
}

pub(crate) fn default_init_config_path(env_resolver: &dyn EnvResolver, cwd: &Path) -> PathBuf {
    standard_config_path(env_resolver).unwrap_or_else(|| cwd.join(DEFAULT_CONFIG_FILE))
}

fn standard_config_path(env_resolver: &dyn EnvResolver) -> Option<PathBuf> {
    if let Some(xdg) = env_resolver.resolve(XDG_CONFIG_HOME_ENV) {
        return Some(PathBuf::from(xdg).join("belter").join(DEFAULT_CONFIG_FILE));
    }

    env_resolver.resolve(HOME_ENV).map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("belter")
            .join(DEFAULT_CONFIG_FILE)
    })
}

fn dotenv_override_path() -> Option<PathBuf> {
    std::env::var(BELTER_ENV_FILE_ENV).ok().map(PathBuf::from)
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
    fn load_if_present(&self, _config_path: &std::path::Path) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DotenvLoader, ProcessDotenvLoader, default_init_config_path, resolve_config_path};
    use infractl_core::config::DEFAULT_CONFIG_FILE;
    use infractl_core::env::FixedEnvResolver;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_fixture_dir() -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("belter-dotenv-test-{ts}"))
    }

    #[test]
    fn loads_dotenv_from_config_directory() {
        let fixture_dir = unique_fixture_dir();
        fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");
        let config_path = fixture_dir.join("belter.toml");
        let env_path = fixture_dir.join(".env");
        fs::write(&config_path, "version = 1\n").expect("config file should be written");
        fs::write(&env_path, "BELTER_DOTENV_TEST=from-config-dir\n")
            .expect("env file should be written");

        unsafe {
            std::env::remove_var("BELTER_DOTENV_TEST");
        }
        ProcessDotenvLoader
            .load_if_present(&config_path)
            .expect("dotenv loading should succeed");
        let value =
            std::env::var("BELTER_DOTENV_TEST").expect("env var should be loaded from config dir");
        assert_eq!(value, "from-config-dir");

        unsafe {
            std::env::remove_var("BELTER_DOTENV_TEST");
        }
        fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
    }

    #[test]
    fn resolve_config_path_prefers_explicit_flag() {
        let cwd = PathBuf::from("/tmp/current");
        let env = FixedEnvResolver::new(HashMap::from([
            (
                "BELTER_CONFIG".to_string(),
                "/tmp/from-env.toml".to_string(),
            ),
            ("XDG_CONFIG_HOME".to_string(), "/tmp/xdg".to_string()),
        ]));

        let resolved = resolve_config_path(Some(Path::new("/tmp/from-flag.toml")), &env, &cwd);

        assert_eq!(resolved, PathBuf::from("/tmp/from-flag.toml"));
    }

    #[test]
    fn resolve_config_path_prefers_belter_config_env_over_discovery() {
        let cwd = PathBuf::from("/tmp/current");
        let env = FixedEnvResolver::new(HashMap::from([
            (
                "BELTER_CONFIG".to_string(),
                "/tmp/from-env.toml".to_string(),
            ),
            ("XDG_CONFIG_HOME".to_string(), "/tmp/xdg".to_string()),
        ]));

        let resolved = resolve_config_path(None, &env, &cwd);

        assert_eq!(resolved, PathBuf::from("/tmp/from-env.toml"));
    }

    #[test]
    fn resolve_config_path_uses_existing_xdg_config_before_local_fallback() {
        let fixture_dir = unique_fixture_dir();
        let xdg_dir = fixture_dir.join("xdg");
        let config_dir = xdg_dir.join("belter");
        fs::create_dir_all(&config_dir).expect("xdg config dir should be created");
        let xdg_config = config_dir.join(DEFAULT_CONFIG_FILE);
        fs::write(&xdg_config, "version = 1\n").expect("xdg config should be written");

        let env = FixedEnvResolver::new(HashMap::from([(
            "XDG_CONFIG_HOME".to_string(),
            xdg_dir.to_str().expect("utf8 path").to_string(),
        )]));

        let resolved = resolve_config_path(None, &env, Path::new("/tmp/current"));

        assert_eq!(resolved, xdg_config);
        fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
    }

    #[test]
    fn resolve_config_path_falls_back_to_local_project_file() {
        let cwd = PathBuf::from("/tmp/current");
        let env = FixedEnvResolver::new(HashMap::new());

        let resolved = resolve_config_path(None, &env, &cwd);

        assert_eq!(resolved, cwd.join(DEFAULT_CONFIG_FILE));
    }

    #[test]
    fn default_init_config_path_prefers_standard_location() {
        let env = FixedEnvResolver::new(HashMap::from([(
            "HOME".to_string(),
            "/tmp/home".to_string(),
        )]));

        let resolved = default_init_config_path(&env, Path::new("/tmp/current"));

        assert_eq!(
            resolved,
            PathBuf::from("/tmp/home/.config/belter").join(DEFAULT_CONFIG_FILE)
        );
    }
}
