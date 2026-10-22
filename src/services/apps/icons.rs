use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::data_dirs;

pub fn read_icon_folders() -> Vec<Vec<PathBuf>> {
    let mut icon_folders = Vec::new();

    let mut roots = Vec::new();

    for dir in data_dirs::find() {
        roots.push(dir.join("icons"));
    }

    // the user's theme first, hicolor is where programs put icons of their own
    let mut themes = match user_theme() {
        Some(theme) => inherited(&theme, &roots),
        None => Vec::new(),
    };

    if !themes.iter().any(|theme| theme == "hicolor") {
        themes.push(String::from("hicolor"));
    }

    for theme in &themes {
        for root in &roots {
            let theme_icon_folders = read_theme_icon_folders(&root.join(theme));

            if theme_icon_folders.is_empty() {
                continue;
            }

            icon_folders.push(theme_icon_folders)
        }
    }

    // loose icons that belong to no theme
    for dir in data_dirs::find() {
        icon_folders.push(vec![dir.join("pixmaps")]);
    }

    icon_folders
}

/*
 * the theme and every theme it inherits from, nearest first; a theme lists
 * them in its index.theme, like "Inherits=Papirus-Dark,hicolor"
 */
fn inherited(theme: &str, roots: &[PathBuf]) -> Vec<String> {
    let mut themes = vec![String::from(theme)];

    let mut next = 0;

    while next < themes.len() {
        let current = themes[next].clone();

        next += 1;

        for parent in parents(&current, roots) {
            if !themes.contains(&parent) {
                themes.push(parent);
            }
        }
    }

    themes
}

// the themes one theme names in its Inherits line, from the first folder that has it
fn parents(theme: &str, roots: &[PathBuf]) -> Vec<String> {
    for root in roots {
        let Ok(index) = fs::read_to_string(root.join(theme).join("index.theme")) else {
            continue;
        };

        for line in index.lines() {
            let Some(list) = line.strip_prefix("Inherits=") else {
                continue;
            };

            return list
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(String::from)
                .collect();
        }

        return Vec::new();
    }

    Vec::new()
}

/*
 * an svg stays sharp at any size, so it beats every png; among svgs the
 * scalable or bigger drawing wins, since the small ones are simplified
 */
fn score(path: &Path) -> u32 {
    let is_svg = path.extension().is_some_and(|extension| extension == "svg");

    if !is_svg {
        return 1 + size(path);
    }

    let size = match size(path) {
        0 => 512,
        size => size,
    };

    10_000 + size
}

// the size is the folder name in themes, like icons/hicolor/48x48/apps
fn size(path: &Path) -> u32 {
    for part in path.components() {
        let Some(part) = part.as_os_str().to_str() else {
            continue;
        };

        let Some((width, _)) = part.split_once('x') else {
            continue;
        };

        if let Ok(width) = width.parse() {
            return width;
        }
    }

    0
}

// the theme gtk is set to, which most desktops share
fn user_theme() -> Option<String> {
    let config = match env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env::var("HOME").ok()?).join(".config"),
    };

    let settings = fs::read_to_string(config.join("gtk-3.0/settings.ini")).unwrap_or_default();

    for line in settings.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if key.trim() == "gtk-icon-theme-name" {
            return Some(value.trim().to_string());
        }
    }

    dconf_theme()
}

// where gnome and gtk 4 keep it when gtk 3's file doesn't say, like "'Papirus-Dark'"
fn dconf_theme() -> Option<String> {
    let output = Command::new("dconf")
        .args(["read", "/org/gnome/desktop/interface/icon-theme"])
        .stderr(Stdio::null())
        .output()
        .ok()?;

    let text = String::from_utf8(output.stdout).ok()?;

    let theme = text.trim().trim_matches('\'');

    if theme.is_empty() {
        return None;
    }

    Some(String::from(theme))
}

fn read_index_theme(theme: &Path) -> String {
    fs::read_to_string(theme.join("index.theme")).unwrap_or_default()
}

fn parse_icon_folder_names(index_theme: &str) -> Vec<&str> {
    let mut icon_folder_names = Vec::new();

    for line in index_theme.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if !matches!(key, "Directories" | "ScaledDirectories") {
            continue;
        }

        for icon_folder_name in value.split(',') {
            let icon_folder_name = icon_folder_name.trim();

            // a comma at the end of the line leaves an empty piece
            if icon_folder_name.is_empty() {
                continue;
            }

            icon_folder_names.push(icon_folder_name);
        }
    }

    icon_folder_names
}

fn read_theme_icon_folders(theme: &Path) -> Vec<PathBuf> {
    let index_theme = read_index_theme(theme);

    let mut theme_icon_folders = Vec::new();

    for icon_folder_name in parse_icon_folder_names(&index_theme) {
        theme_icon_folders.push(theme.join(icon_folder_name));
    }

    theme_icon_folders
}

fn look_up_icon_file_in_folder(icon_name: &str, icon_folder: &Path) -> Option<PathBuf> {
    for extension in ["svg", "png"] {
        let icon_file = icon_folder.join(format!("{icon_name}.{extension}"));

        if icon_file.is_file() {
            return Some(icon_file);
        }
    }

    None
}

// a theme keeps the same icon in many sizes, so every folder is checked for the sharpest one
fn look_up_icon_file_in_theme(icon_name: &str, theme_icon_folders: &[PathBuf]) -> Option<PathBuf> {
    let mut best_icon_file: Option<PathBuf> = None;

    for icon_folder in theme_icon_folders {
        let Some(icon_file) = look_up_icon_file_in_folder(icon_name, icon_folder) else {
            continue;
        };

        let is_better = match &best_icon_file {
            Some(best) => score(&icon_file) > score(best),
            None => true,
        };

        if is_better {
            best_icon_file = Some(icon_file);
        }
    }

    best_icon_file
}

// the user's theme first, so a later theme only fills in icons it lacks
pub fn look_up_icon_file(icon_name: &str, icon_folders: &[Vec<PathBuf>]) -> Option<PathBuf> {
    for theme_icon_folders in icon_folders {
        let Some(icon_file) = look_up_icon_file_in_theme(icon_name, theme_icon_folders) else {
            continue;
        };

        return Some(icon_file);
    }

    None
}
