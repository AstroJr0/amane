use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{library, paths};

// the binary cargo builds from the user's config
pub const BINARY: &str = "amane-shell";

// unpacks the library and writes the cargo project around the user's main.rs
pub fn prepare() -> Option<PathBuf> {
    let cache = paths::cache();

    let library_folder = cache.join("library");
    let project_folder = cache.join("project");

    library::unpack(&library_folder);

    fs::create_dir_all(&project_folder).expect("failed to create project folder");

    let config = paths::config();
    let source = config.join("src");
    let main = source.join("main.rs");

    let main_path = main.display().to_string();
    let library_path = library_folder.display().to_string();

    let config_manifest_path = config.join("Cargo.toml");

    let contents = match fs::read_to_string(&config_manifest_path) {
        Ok(contents) => contents,

        Err(err) if err.kind() == io::ErrorKind::NotFound => default_manifest(&library_path),

        Err(err) => {
            eprintln!("failed to read {}: {err}", config_manifest_path.display());

            return None;
        }
    };

    let mut user_doc: toml_edit::DocumentMut = match contents.parse() {
        Ok(doc) => doc,
        Err(err) => {
            eprintln!("failed to parse {}: {err}", config_manifest_path.display());

            return None;
        }
    };

    // rust-analyzer only finds amane through a Cargo.toml next to the user's src
    set_amane_dependency(&mut user_doc, &library_path);

    user_doc
        .entry("workspace")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

    write_if_changed(&config_manifest_path, user_doc.to_string().as_bytes());

    let project_manifest = build_project_manifest(&user_doc, &main_path, &library_path, &config);

    write_if_changed(
        &project_folder.join("Cargo.toml"),
        project_manifest.to_string().as_bytes(),
    );

    // the library's own lock file pins the versions amane was tested with
    let lock = fs::read(library_folder.join("Cargo.lock")).expect("failed to read library lock");

    let lock_changed = clean_if_dependencies_changed(&lock, &project_folder);

    let project_lock = project_folder.join("Cargo.lock");

    // the user's lock wins, so their crates keep the versions they were built with
    match fs::read(config.join("Cargo.lock")) {
        Ok(user_lock) => write_if_changed(&project_lock, &user_lock),

        Err(_) if lock_changed || !project_lock.exists() => {
            write_if_changed(&project_lock, &lock);
        }

        Err(_) => {}
    }

    rebuild_if_swapped(&config, &source, &main, &project_folder);

    Some(project_folder)
}

// keeps the lock file in the user's shell folder in sync when one exists
pub fn sync_lock(config: &Path, project: &Path) {
    let config_lock = config.join("Cargo.lock");
    let project_lock = project.join("Cargo.lock");

    if config_lock.exists() {
        if let Ok(contents) = fs::read(&project_lock) {
            write_if_changed(&config_lock, &contents);
        }
    }
}

/*
 * cargo only rebuilds when a source is newer than the last build, so a config
 * moved or restored with its old timestamps would keep running the previous one.
 * hashing the contents catches that, and touching main.rs makes cargo notice
 */
fn rebuild_if_swapped(config: &Path, source: &Path, main: &Path, project: &Path) {
    let mut hasher = DefaultHasher::new();

    hash_folder(source, source, &mut hasher);

    if let Ok(manifest_bytes) = fs::read(config.join("Cargo.toml")) {
        manifest_bytes.hash(&mut hasher);
    }

    if let Ok(lock_bytes) = fs::read(config.join("Cargo.lock")) {
        lock_bytes.hash(&mut hasher);
    }

    if !update_stamp(&project.join("source-hash"), hasher.finish()) {
        return;
    }

    if let Ok(file) = fs::File::options().append(true).open(main) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/*
 * cargo names every build output after a hash of its dependencies and settings,
 * so a new lock leaves all the old outputs behind and target grows forever.
 * source edits reuse the same names, so only a new lock needs a clean.
 * cargo rewrites the project's Cargo.lock itself, so the library's lock is
 * remembered as a hash instead of compared directly
 */
fn clean_if_dependencies_changed(lock: &[u8], project: &Path) -> bool {
    let mut hasher = DefaultHasher::new();

    lock.hash(&mut hasher);

    if !update_stamp(&project.join("lock-hash"), hasher.finish()) {
        return false;
    }

    // target is missing before the first build, so a failure here is fine
    let _ = fs::remove_dir_all(project.join("target"));

    true
}

// remembers the hash in the stamp file, true when it differs from the last one
fn update_stamp(stamp: &Path, hash: u64) -> bool {
    let hash = hash.to_string();

    if fs::read_to_string(stamp).is_ok_and(|previous| previous == hash) {
        return false;
    }

    write_if_changed(stamp, hash.as_bytes());

    true
}

// sorted so the same files always give the same hash
fn hash_folder(root: &Path, folder: &Path, hasher: &mut DefaultHasher) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };

    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();

    paths.sort();

    for path in paths {
        path.strip_prefix(root).unwrap_or(&path).hash(hasher);

        if path.is_dir() {
            hash_folder(root, &path, hasher);
        } else if let Ok(contents) = fs::read(&path) {
            contents.hash(hasher);
        }
    }
}

/*
 * the dev build (the only one) optimises amane and its dependencies, drawing unoptimised
 * is too slow for smooth animation; only the user's own crate stays
 * unoptimised, so a save still rebuilds quickly after the first build.
 * dependencies carry no debug info and the user's crate only line tables,
 * which keeps the binary small and the link on every save short.
 * dependencies skip incremental, they only rebuild when amane updates.
 * the empty workspace keeps cargo from joining a workspace in a parent folder
 */
fn default_manifest(library: &str) -> String {
    format!(
        "[package]
name = \"{BINARY}\"
version = \"0.1.0\"
edition = \"2024\"

[dependencies]
amane = {{ path = '{library}' }}
# you can also add extra crates, for example:
# chrono = \"0.4\"

[profile.dev]
debug = \"line-tables-only\"

[profile.dev.package.\"*\"]
opt-level = 3
debug = false
incremental = false

[workspace]
"
    )
}

fn build_project_manifest(
    user_doc: &toml_edit::DocumentMut,
    main_path: &str,
    library_path: &str,
    config: &Path,
) -> toml_edit::DocumentMut {
    let mut doc = user_doc.clone();

    doc.entry("package")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

    doc["package"]["name"] = toml_edit::value(BINARY);
    doc["package"]["edition"] = toml_edit::value("2024");

    if doc["package"].get("version").is_none() {
        doc["package"]["version"] = toml_edit::value("0.1.0");
    }

    doc.remove("bin");

    let mut bin_table = toml_edit::Table::new();
    bin_table.insert("name", toml_edit::value(BINARY));
    bin_table.insert("path", toml_edit::value(main_path));

    let mut bin_array = toml_edit::ArrayOfTables::new();
    bin_array.push(bin_table);
    doc["bin"] = toml_edit::Item::ArrayOfTables(bin_array);

    set_amane_dependency(&mut doc, library_path);

    make_paths_absolute(doc.as_item_mut(), config);

    ensure_profiles(&mut doc);

    doc["workspace"] = toml_edit::Item::Table(toml_edit::Table::new());

    doc
}

// amane always comes from the unpacked library, whatever the user's manifest says
fn set_amane_dependency(doc: &mut toml_edit::DocumentMut, library_path: &str) {
    let mut amane = toml_edit::InlineTable::new();

    amane.insert("path", library_path.into());

    let dependencies = doc
        .entry("dependencies")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

    if let Some(table) = dependencies.as_table_like_mut() {
        table.insert(
            "amane",
            toml_edit::Item::Value(toml_edit::Value::InlineTable(amane)),
        );
    }
}

fn resolve_relative(base: &Path, rel: &Path) -> PathBuf {
    let mut result = base.to_path_buf();

    for component in rel.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::Normal(c) => {
                result.push(c);
            }
            _ => {}
        }
    }

    result
}

// every relative `path` in the manifest is read from the shell folder, not from the generated project
fn make_paths_absolute(item: &mut toml_edit::Item, base_dir: &Path) {
    if let Some(array) = item.as_array_of_tables_mut() {
        for table in array.iter_mut() {
            for (_, child) in table.iter_mut() {
                make_paths_absolute(child, base_dir);
            }
        }

        return;
    }

    let Some(table) = item.as_table_like_mut() else {
        return;
    };

    if let Some(path) = table
        .get_mut("path")
        .and_then(toml_edit::Item::as_value_mut)
    {
        make_absolute(path, base_dir);
    }

    for (_, child) in table.iter_mut() {
        make_paths_absolute(child, base_dir);
    }
}

fn make_absolute(value: &mut toml_edit::Value, base_dir: &Path) {
    let Some(relative) = value.as_str().filter(|path| Path::new(path).is_relative()) else {
        return;
    };

    let absolute = resolve_relative(base_dir, Path::new(relative));

    *value = absolute.display().to_string().into();
}

fn ensure_profiles(doc: &mut toml_edit::DocumentMut) {
    if doc.get("profile").is_none() {
        doc["profile"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    if doc["profile"].get("dev").is_none() {
        doc["profile"]["dev"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    doc["profile"]["dev"]["debug"] = toml_edit::value("line-tables-only");

    if doc["profile"]["dev"].get("package").is_none() {
        doc["profile"]["dev"]["package"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    if doc["profile"]["dev"]["package"].get("*").is_none() {
        doc["profile"]["dev"]["package"]["*"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    doc["profile"]["dev"]["package"]["*"]["opt-level"] = toml_edit::value(3);
    doc["profile"]["dev"]["package"]["*"]["debug"] = toml_edit::value(false);
    doc["profile"]["dev"]["package"]["*"]["incremental"] = toml_edit::value(false);
}

fn write_if_changed(path: &Path, contents: &[u8]) {
    if fs::read(path).is_ok_and(|current| current == contents) {
        return;
    }

    fs::write(path, contents).expect("failed to write project file");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_amane_dependency_keeps_other_crates() {
        let manifest = r#"[package]
name = "my-awesome-shell"
version = "0.1.0"
edition = "2024"

[dependencies]
chrono = "0.4"
serde = { version = "1.0", features = ["derive"] }
"#;

        let mut doc: toml_edit::DocumentMut = manifest.parse().unwrap();
        set_amane_dependency(&mut doc, "/path/to/library");

        let output = doc.to_string();
        assert!(output.contains(r#"chrono = "0.4""#));
        assert!(output.contains(r#"serde = { version = "1.0", features = ["derive"] }"#));
        assert!(output.contains(r#"amane = { path = "/path/to/library" }"#));
    }

    #[test]
    fn the_amane_dependency_path_is_replaced() {
        let manifest = r#"[dependencies]
amane = { path = "/old/path/to/library" }
chrono = "0.4"
"#;

        let mut doc: toml_edit::DocumentMut = manifest.parse().unwrap();
        set_amane_dependency(&mut doc, "/new/path/to/library");

        let output = doc.to_string();
        assert!(output.contains(r#"amane = { path = "/new/path/to/library" }"#));
        assert!(!output.contains("/old/path/to/library"));
        assert!(output.contains(r#"chrono = "0.4""#));
    }

    #[test]
    fn test_build_project_manifest_resolves_relative_paths() {
        let manifest = r#"[package]
name = "my-custom-shell"
version = "0.2.0"
edition = "2024"

[dependencies]
chrono = "0.4"
local_crate = { path = "./subcrates/local_crate" }

[dependencies.another_local]
path = "../another_local"
"#;

        let doc: toml_edit::DocumentMut = manifest.parse().unwrap();
        let base_dir = Path::new("/home/user/.config/amane");
        let project_doc = build_project_manifest(
            &doc,
            "/home/user/.config/amane/src/main.rs",
            "/home/user/.cache/amane/library",
            base_dir,
        );

        let output = project_doc.to_string();
        assert!(output.contains(r#"name = "amane-shell""#));
        assert!(output.contains(r#"path = "/home/user/.config/amane/src/main.rs""#));
        assert!(output.contains(r#"amane = { path = "/home/user/.cache/amane/library" }"#));
        assert!(output.contains(r#"chrono = "0.4""#));
        assert!(output.contains("/home/user/.config/amane/subcrates/local_crate"));
        assert!(output.contains("/home/user/.config/another_local"));
        assert!(output.contains("[profile.dev]"));
        assert!(output.contains("[profile.dev.package.\"*\"]"));
        assert!(output.contains("[workspace]"));
    }

    #[test]
    fn test_comments_and_formatting_preserved() {
        let manifest = r#"# My custom shell configuration
[package]
name = "my-awesome-shell"
version = "0.1.0"
edition = "2024"

[dependencies]
chrono = "0.4"
serde = { version = "1.0", features = ["derive"] }
"#;

        let mut doc: toml_edit::DocumentMut = manifest.parse().unwrap();
        set_amane_dependency(&mut doc, "/cache/library");

        let output = doc.to_string();
        assert!(output.contains("# My custom shell configuration"));
        assert!(output.contains("# Standard date/time library"));
        assert!(output.contains("# Serialization"));
        assert!(output.contains(r#"amane = { path = "/cache/library" }"#));
    }

    #[test]
    fn test_target_dependencies_and_expanded_amane_table() {
        let manifest = r#"[package]
name = "my-custom-shell"
version = "0.1.0"
edition = "2024"

[dependencies.amane]
path = "/old/library"

[target.'cfg(unix)'.dependencies]
nix = "0.29"
local_unix = { path = "./unix_helper" }
"#;

        let mut doc: toml_edit::DocumentMut = manifest.parse().unwrap();
        set_amane_dependency(&mut doc, "/new/library");

        let base_dir = Path::new("/home/user/.config/amane");
        let project_doc = build_project_manifest(
            &doc,
            "/home/user/.config/amane/src/main.rs",
            "/new/library",
            base_dir,
        );

        let output = project_doc.to_string();
        assert!(output.contains(r#"path = "/new/library""#));
        assert!(output.contains(r#"nix = "0.29""#));
        assert!(output.contains("/home/user/.config/amane/unix_helper"));
    }

    #[test]
    fn test_prepare_flow_with_user_dependencies() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("amane_test_{now}"));
        let config_dir = temp_dir.join("config");
        let cache_dir = temp_dir.join("cache");

        fs::create_dir_all(config_dir.join("src")).unwrap();
        fs::write(config_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        // set AMANE_CONFIG and XDG_CACHE_HOME for the test
        unsafe {
            std::env::set_var("AMANE_CONFIG", &config_dir);
            std::env::set_var("XDG_CACHE_HOME", &cache_dir);
        }

        let project_dir = prepare().expect("prepare failed on initial call");

        let config_toml = fs::read_to_string(config_dir.join("Cargo.toml")).unwrap();
        assert!(config_toml.contains("amane = { path = "));

        // user adds extra crate (test with the chrono crate)
        let updated_config_toml = format!("{config_toml}\nchrono = \"0.4\"\n");
        fs::write(config_dir.join("Cargo.toml"), updated_config_toml).unwrap();

        let _ = prepare().expect("prepare failed on second call");

        let project_toml = fs::read_to_string(project_dir.join("Cargo.toml")).unwrap();
        assert!(project_toml.contains(r#"chrono = "0.4""#));
        assert!(project_toml.contains("amane = { path = "));
        assert!(project_toml.contains("[profile.dev]"));

        // clean up
        unsafe {
            std::env::remove_var("AMANE_CONFIG");
            std::env::remove_var("XDG_CACHE_HOME");
        }
        let _ = fs::remove_dir_all(temp_dir);
    }
}
