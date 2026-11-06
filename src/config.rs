//! The settings file, `~/.config/whypkg/config.toml`. It holds one setting so
//! far: which sources whypkg leaves unread. The browser's settings box (`o`)
//! writes it, and a person may edit it too, so writing keeps their comments.

use crate::model::Source;
use std::path::PathBuf;
use toml_edit::{Array, DocumentMut, value};

/// What the file says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// Sources whypkg does not read, in the order the file lists them.
    pub skip: Vec<Source>,
}

/// `$XDG_CONFIG_HOME/whypkg/config.toml`, or `~/.config/whypkg/config.toml`.
pub fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::backend::home().map(|h| h.join(".config")))
        .map(|d| d.join("whypkg/config.toml"))
}

/// The path as a person writes it, with `~` for the home directory.
pub fn shown(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().into_owned();
    match crate::backend::home() {
        Some(home) => match path.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.to_string_lossy()),
            Err(_) => text,
        },
        None => text,
    }
}

/// Read the file. A missing file is the defaults; a broken one is an error
/// that names the file and what to do.
pub fn load() -> Result<Config, String> {
    let Some(path) = path() else {
        return Ok(Config::default());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|e| format!("`{}`: {e}", shown(&path))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(_) => Err(format!(
            "cannot read `{}`: check its permissions",
            shown(&path)
        )),
    }
}

/// Parse the file's text.
pub fn parse(text: &str) -> Result<Config, String> {
    let doc = text
        .parse::<DocumentMut>()
        .map_err(|e| format!("{}, fix it or delete the file", first_line(&e.to_string())))?;
    let mut config = Config::default();
    if let Some(item) = doc.get("skip") {
        let list = item
            .as_array()
            .ok_or("`skip` must be a list of sources, such as `skip = [\"npm\"]`")?;
        for entry in list {
            // A value that is not a string (`3`) is named as it is written.
            let written = entry.to_string();
            let word = entry.as_str().unwrap_or(written.trim());
            let source = Source::from_word(word).ok_or_else(|| {
                format!(
                    "`skip` names `{word}`, which is not a source ({})",
                    Source::words()
                )
            })?;
            if !config.skip.contains(&source) {
                config.skip.push(source);
            }
        }
    }
    Ok(config)
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_string()
}

/// Write `config`, keeping whatever else `existing` (the file as it is now)
/// holds: comments, and keys a newer whypkg may have added.
pub fn render(existing: Option<&str>, config: &Config) -> String {
    let mut doc = existing
        .and_then(|t| t.parse::<DocumentMut>().ok())
        .unwrap_or_else(|| {
            format!(
                "# whypkg's settings. The browser writes this file (`o`).\n\n\
                 # Sources whypkg does not read: {}\n\
                 skip = []\n",
                Source::words()
            )
            .parse()
            .unwrap_or_default()
        });
    let mut list = Array::new();
    for source in &config.skip {
        list.push(source.word());
    }
    match doc.get_mut("skip").and_then(|i| i.as_array_mut()) {
        // Swapping the values in place keeps the comment above the key.
        Some(existing) => {
            existing.clear();
            for source in &config.skip {
                existing.push(source.word());
            }
        }
        None => doc["skip"] = value(list),
    }
    doc.to_string()
}

/// Save `config` and return where it went.
pub fn save(config: &Config) -> Result<PathBuf, String> {
    let path = path().ok_or("`HOME` is not set, so there is nowhere to keep settings")?;
    let existing = std::fs::read_to_string(&path).ok();
    let text = render(existing.as_deref(), config);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|_| format!("cannot create `{}`: check its permissions", shown(dir)))?;
    }
    std::fs::write(&path, text)
        .map_err(|_| format!("cannot write `{}`: check its permissions", shown(&path)))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewriting_the_settings_keeps_comments_and_unknown_keys() {
        let existing = "# mine, keep me\nskip = [\"npm\"] # trailing note\ntheme = \"dark\"\n";
        let config = Config {
            skip: vec![Source::Cargo, Source::Uv],
        };
        let written = render(Some(existing), &config);
        assert!(
            written.contains("# mine, keep me"),
            "comment lost:\n{written}"
        );
        assert!(
            written.contains("theme = \"dark\""),
            "unknown key lost:\n{written}"
        );
        assert_eq!(
            parse(&written).map(|c| c.skip),
            Ok(vec![Source::Cargo, Source::Uv]),
            "the written file does not read back as what was saved:\n{written}"
        );
    }
}
