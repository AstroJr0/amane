use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{self, Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::paths;

pub fn run(folder: Option<&str>) -> ExitCode {
    if is_nix_install() {
        return update_with_nix();
    }

    let repo = match folder {
        Some(folder) => {
            let path = path::absolute(folder).expect("failed to resolve folder");

            if !is_amane_repo(&path) {
                eprintln!("### error: {} is not an amane repository", path.display());

                return ExitCode::FAILURE;
            }

            save(&path);

            path
        }

        None => {
            let Some(path) = find_repo() else {
                eprintln!("### error: failed to find the amane repository");
                eprintln!(
                    "run `amane update <folder>` to point to your clone, or run it inside the clone"
                );

                return ExitCode::FAILURE;
            };

            path
        }
    };

    println!("### info: checking for updates in {}", repo.display());

    if !step("git", &["fetch"], Some(&repo)) {
        return ExitCode::FAILURE;
    }

    match commits_behind(&repo) {
        Some(0) => {
            println!("### info: amane is already up to date!");

            return ExitCode::SUCCESS;
        }

        Some(count) => println!("### info: {count} new commit(s) available"),

        None => {
            eprintln!("### error: could not compare with the upstream branch, does it have one?");

            return ExitCode::FAILURE;
        }
    }

    if !confirm("pull and reinstall? [Y/n] ", true) {
        println!("### info: skipped.");

        return ExitCode::SUCCESS;
    }

    // a fast-forward never makes a merge commit or stops on a conflict
    if !step("git", &["pull", "--ff-only"], Some(&repo)) {
        return ExitCode::FAILURE;
    }

    if !step(
        "cargo",
        &["install", "--locked", "--path", "cli"],
        Some(&repo),
    ) {
        return ExitCode::FAILURE;
    }

    println!("### info: updated amane");
    println!(
        "### tips: the next `amane compile` or `amane dev` rebuilds your shell with the new library"
    );

    ExitCode::SUCCESS
}

// a Nix install is read-only, so it has to be upgraded through Nix
fn update_with_nix() -> ExitCode {
    println!("### info: amane was installed through Nix.");
    println!();
    println!("  nix profile:  nix profile upgrade amane");
    println!("  flake:        update the flake input and rebuild");
    println!();

    if !confirm("run `nix profile upgrade amane` now? [y/N] ", false) {
        println!("### info: skipped. update manually when you're ready.");

        return ExitCode::SUCCESS;
    }

    if !step("nix", &["profile", "upgrade", "amane"], None) {
        return ExitCode::FAILURE;
    }

    println!("### info: amane updated!");

    ExitCode::SUCCESS
}

fn is_nix_install() -> bool {
    env::current_exe().is_ok_and(|exe| exe.starts_with("/nix/store"))
}

// runs a command, in the folder when given, true when it succeeds
fn step(program: &str, args: &[&str], folder: Option<&Path>) -> bool {
    println!("### running: {program} {}", args.join(" "));

    let mut command = Command::new(program);

    command.args(args);

    if let Some(folder) = folder {
        command.current_dir(folder);
    }

    match command.status() {
        Ok(status) if status.success() => true,

        Ok(status) => {
            eprintln!("### error: {program} failed with {status}");

            false
        }

        Err(err) => {
            eprintln!("### error: failed to run {program}: {err}");

            false
        }
    }
}

// how many commits the upstream branch has that this one lacks, none when there is no upstream
fn commits_behind(repo: &Path) -> Option<u32> {
    let output = Command::new("git")
        .args(["rev-list", "--count", "HEAD..@{upstream}"])
        .current_dir(repo)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

// an empty answer, like a closed stdin, takes the default
fn confirm(question: &str, default: bool) -> bool {
    print!("{question}");

    let _ = io::stdout().flush();

    let mut answer = String::new();

    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }

    match answer.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "y" | "yes" | "yep" => true,
        _ => false,
    }
}

fn find_repo() -> Option<PathBuf> {
    let from_env = env::var_os("AMANE_REPO").map(PathBuf::from);

    if let Some(folder) = from_env.filter(|folder| is_amane_repo(folder)) {
        return Some(folder);
    }

    let current = env::current_dir().ok();

    if let Some(folder) = current.filter(|folder| is_amane_repo(folder)) {
        save(&folder);

        return Some(folder);
    }

    let saved = fs::read_to_string(paths::repo_file())
        .ok()
        .map(|saved| PathBuf::from(saved.trim_end()));

    if let Some(folder) = saved.filter(|folder| is_amane_repo(folder)) {
        return Some(folder);
    }

    let home = paths::home().join("amane");

    if is_amane_repo(&home) {
        save(&home);

        return Some(home);
    }

    None
}

fn is_amane_repo(path: &Path) -> bool {
    path.join(".git").exists()
        && path.join("cli").join("Cargo.toml").exists()
        && path.join("src").join("lib.rs").exists()
}

// only a convenience for the next run, so a failure to remember is not worth stopping for
fn save(folder: &Path) {
    let file = paths::repo_file();

    if let Some(parent) = file.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let _ = fs::write(&file, folder.display().to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_needs_git_the_cli_and_the_library() {
        let folder = env::temp_dir().join(format!("amane-update-test-{}", std::process::id()));

        fs::create_dir_all(folder.join(".git")).unwrap();
        fs::create_dir_all(folder.join("cli")).unwrap();
        fs::create_dir_all(folder.join("src")).unwrap();

        assert!(!is_amane_repo(&folder));

        fs::write(folder.join("cli").join("Cargo.toml"), "").unwrap();
        fs::write(folder.join("src").join("lib.rs"), "").unwrap();

        assert!(is_amane_repo(&folder));

        fs::remove_dir_all(folder.join(".git")).unwrap();

        assert!(!is_amane_repo(&folder));

        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn an_empty_folder_is_not_a_repository() {
        assert!(!is_amane_repo(Path::new("/nonexistent/amane")));
    }
}
