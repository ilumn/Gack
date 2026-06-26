use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub ui: UiConfig,
    pub git: GitConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiConfig {
    pub mouse: MouseMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitConfig {
    pub auto_refresh_ms: Option<u64>,
    pub filesystem_watch: WatchMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    Auto,
    Always,
    Never,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig {
                mouse: MouseMode::Auto,
            },
            git: GitConfig {
                auto_refresh_ms: Some(2500),
                filesystem_watch: WatchMode::Auto,
            },
        }
    }
}

impl Config {
    pub fn load(path: Option<PathBuf>, no_config: bool) -> Result<Self, String> {
        Self::load_with_project(path, no_config, None)
    }

    pub fn load_with_project(
        path: Option<PathBuf>,
        no_config: bool,
        repo_root: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let mut config = Self::default();
        if no_config {
            return Ok(config);
        }

        let explicit = path.is_some();
        let path = path.or_else(default_config_path);
        if let Some(path) = path {
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .map_err(|err| format!("could not read config {}: {err}", path.display()))?;
                parse_config_text(&text, &mut config)
                    .map_err(|err| format!("invalid config {}: {err}", path.display()))?;
            } else if explicit {
                return Err(format!("config {} does not exist", path.display()));
            }
        }

        if !explicit && let Some(repo_root) = repo_root {
            let project_path = repo_root.join(".gack.toml");
            if project_path.exists() {
                let text = std::fs::read_to_string(&project_path).map_err(|err| {
                    format!("could not read config {}: {err}", project_path.display())
                })?;
                parse_config_text(&text, &mut config)
                    .map_err(|err| format!("invalid config {}: {err}", project_path.display()))?;
            }
        }
        Ok(config)
    }
}

fn default_config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("GACK_CONFIG") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    {
        Some(home.join("Library/Application Support/gack/config.toml"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Some(base.join("gack/config.toml"))
    }
}

fn parse_config_text(text: &str, config: &mut Config) -> Result<(), String> {
    let mut section = String::new();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {} is not key = value", line_index + 1));
        };
        apply_config_value(&section, key.trim(), value.trim(), config)?;
    }
    Ok(())
}

fn apply_config_value(
    section: &str,
    key: &str,
    value: &str,
    config: &mut Config,
) -> Result<(), String> {
    match (section, key) {
        ("ui", "mouse") => {
            config.ui.mouse = match string_value(value)?.as_str() {
                "auto" => MouseMode::Auto,
                "always" => MouseMode::Always,
                "never" => MouseMode::Never,
                other => return Err(format!("unsupported mouse mode {other}")),
            };
        }
        ("git", "auto_refresh_ms") => {
            config.git.auto_refresh_ms = if value == "false" || value == "0" {
                None
            } else {
                Some(value.parse().map_err(|_| "invalid auto_refresh_ms")?)
            };
        }
        ("git", "filesystem_watch") => {
            config.git.filesystem_watch = match string_value(value)?.as_str() {
                "auto" => WatchMode::Auto,
                "always" => WatchMode::Always,
                "never" => WatchMode::Never,
                other => return Err(format!("unsupported filesystem_watch mode {other}")),
            };
        }
        _ => {}
    }
    Ok(())
}

fn string_value(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        Ok(value[1..value.len() - 1].to_string())
    } else {
        Err(format!("expected quoted string, got {value}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_config_subset() {
        let mut config = Config::default();
        parse_config_text(
            r#"
[ui]
mouse = "never"

[git]
auto_refresh_ms = 1500
filesystem_watch = "always"
"#,
            &mut config,
        )
        .unwrap();
        assert_eq!(config.ui.mouse, MouseMode::Never);
        assert_eq!(config.git.auto_refresh_ms, Some(1500));
        assert_eq!(config.git.filesystem_watch, WatchMode::Always);
    }

    #[test]
    fn obsolete_ai_config_is_ignored() {
        let mut config = Config::default();
        parse_config_text(
            r#"
[ui]
mouse = "never"

[ai.commit]
command = "danger"
"#,
            &mut config,
        )
        .unwrap();
        assert_eq!(config.ui.mouse, MouseMode::Never);
    }
}
