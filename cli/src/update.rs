use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{self, Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::paths;

pub fn run(folder: Option<&str>) -> ExitCode {
    if is_nix_install() {
        return run_nix_update();
    }

    let repo = match folder {
        Some(folder) => {
            let path = path::absolute(folder).expect("failed to resolve folder");

            if !is_amane_repo(&path) {
                eprintln!(
                    "### error: failed to update: {} is not an amane repository",
                    path.display()
                );

                return ExitCode::FAILURE;
            }

            save(&path);

            path
        }

        None => match find_repo() {
            Some(path) => path,

            None => {
                eprintln!("### error: failed to find amane repository");
                eprintln!(
                    "run `amane target <folder>` to point to your clone, or run it inside the clone"
                );

                return ExitCode::FAILURE;
            }
        },
    };
    
    // run `git fetch` to compare the HEAD
    println!("### info: checking for updates in {}", repo.display());
    println!("### running: git fetch");
        
    let Ok(fetch_status) = Command::new("git")
        .arg("fetch")
        .current_dir(&repo)
        .status()
    else {
        eprintln!("### error: failed to run git fetch");
        return ExitCode::FAILURE;
    };

    if !fetch_status.success() {
        eprintln!("### error: git fetch failed");
        return ExitCode::FAILURE;
    }

    // compare the current HEAD and the pulled HEAD
    let local = git_rev(&repo, "HEAD");
    let remote = git_rev(&repo, "FETCH_HEAD");

    match (local, remote) {
        (Some(l), Some(r)) if l == r => {
            println!("### info: amane is already up to date!");
            return ExitCode::SUCCESS;
        }

        (None, _) | (_, None) => {
            eprintln!("### error: could not read git revisions, continuing anyway");
        }

        _ => {}
    }

    println!("### info: updates available");
    print!("pull and reinstall? [Y/n] ");

    let _ = io::stdout().flush();

    let mut input = String::new();

    if io::stdin().read_line(&mut input).is_err() {
        eprintln!("### error: failed to read input");
        return ExitCode::FAILURE;
    }

    let answer = input.trim().to_ascii_lowercase();

    if answer == "n" || answer == "no" || answer == "nope" {
        println!("### info: skipped.");
        return ExitCode::SUCCESS;
    }

    println!("### info: pulling {}", repo.display());
    println!(" ");
    println!("### running: git pull");
    
    let Ok(pull_status) = Command::new("git")
        .arg("pull")
        .current_dir(&repo)
        .status()
    else {
        eprintln!("### error: failed to run git pull");
        return ExitCode::FAILURE;
    };

    if !pull_status.success() {
        eprintln!("### error: git pull failed");
        return ExitCode::FAILURE;
    }

    println!("### info: installing amane");
    
    /*
    * check if the folder contains cli folder and Cargo.toml inside it
    * or its assumed its alread in cli folder
    * as pulled repo will always contains a cli folder
    */
    
    let cargo_args = if repo.join("cli").join("Cargo.toml").exists() {
        vec!["install", "--locked", "--path", "cli"]
    } else {
        vec!["install", "--locked", "--path", "."]
    };

    println!("### info: installing amane through the `cli` folder");
    let Ok(install_status) = Command::new("cargo")
        .args(&cargo_args)
        .current_dir(&repo)
        .status()
    else {
        eprintln!("### error: failed to run cargo install");
        return ExitCode::FAILURE;
    };

    if !install_status.success() {
        eprintln!("### error: cargo install failed");
        return ExitCode::FAILURE;
    }

    println!("### info: updated amane");
    println!("### tips: the next `amane compile` or `amane dev` rebuilds your shell with the new library");

    ExitCode::SUCCESS
}

// well, nix repos "can" be installed both in current profile
// and system wide
fn is_nix_install() -> bool {
    let Ok(exe) = env::current_exe() else {
        return false;
    };

    exe.starts_with("/nix/store")
}

fn git_rev(repo: &Path, rev: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(repo)
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn find_repo() -> Option<PathBuf> {
    if let Some(folder) = env::var_os("AMANE_REPO").map(PathBuf::from) {
        if is_amane_repo(&folder) {
            return Some(folder);
        }
    }

    if let Ok(current) = env::current_dir() {
        if is_amane_repo(&current) {
            save(&current);

            return Some(current);
        }
    }

    if let Ok(saved) = fs::read_to_string(paths::repo_file()) {
        let saved_path = PathBuf::from(saved.trim_end());

        if is_amane_repo(&saved_path) {
            return Some(saved_path);
        }
    }

    let home_repo = paths::home().join("amane");

    if is_amane_repo(&home_repo) {
        save(&home_repo);

        return Some(home_repo);
    }

    None
}

fn is_amane_repo(path: &Path) -> bool {
    path.join(".git").exists()
        && path.join("cli").join("Cargo.toml").exists()
        && path.join("src").join("lib.rs").exists()
}

fn save(folder: &Path) {
    let file = paths::repo_file();

    let parent = file.parent().expect("failed to find state folder");

    fs::create_dir_all(parent).expect("failed to create state folder");

    let _ = fs::write(&file, folder.display().to_string());
}

fn run_nix_update() -> ExitCode {
    println!("### info: amane was installed through Nix.");
    println!();
    println!("  nix profile:  nix profile upgrade amane         ");
    println!("  flake:        update the flake input and rebuild");
    println!();
    print!("run `nix profile upgrade amane` now? [y/N] ");

    let _ = io::stdout().flush();

    let mut input = String::new();

    if io::stdin().read_line(&mut input).is_err() {
        // non-interactive, just print the hint
        return ExitCode::SUCCESS;
    }

    let answer = input.trim().to_ascii_lowercase();

    if answer == "y" || answer == "yes" || answer == "yep" {
        println!("### running: nix profile upgrade amane");
        let Ok(status) = Command::new("nix")
            .args(["profile", "upgrade", "amane"])
            .status()
        else {
            eprintln!("### error: failed to run nix profile upgrade");
            return ExitCode::FAILURE;
        };

        if !status.success() {
            eprintln!("### error: nix profile upgrade failed");
            return ExitCode::FAILURE;
        }

        println!("### info: amane updated!");
    } else {
        println!("### info: skipped. update manually when you're ready.");
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_amane_repo() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();

        assert!(is_amane_repo(&repo));
        assert!(!is_amane_repo(Path::new("/tmp")));
    }
        }
        
