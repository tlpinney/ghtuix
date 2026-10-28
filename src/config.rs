//! `~/.config/ghtuix/config.toml` (or `$XDG_CONFIG_HOME/ghtuix/config.toml`).

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// GitHub host (`github.com` or a GitHub Enterprise Server).
    pub host: Option<String>,
    /// Repositories (`owner/name`) never shown in the Inbox.
    pub excluded_repositories: Vec<String>,
}

impl Config {
    pub fn host(&self) -> &str {
        self.host.as_deref().unwrap_or("github.com")
    }

    /// The saved config, or the defaults when there is none; a broken file is an error.
    pub fn load() -> anyhow::Result<Self> {
        let Some(path) = path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|error| anyhow::anyhow!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(anyhow::anyhow!("{}: {error}", path.display())),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("ghtuix").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hidden_repositories_and_refuses_unknown_keys() {
        let config: Config = toml::from_str(r#"excluded_repositories = ["o/a"]"#).unwrap();
        assert_eq!(config.excluded_repositories, ["o/a"]);
        assert_eq!(config.host(), "github.com");
        assert!(toml::from_str::<Config>("colour = 1").is_err());
    }
}
