# The amane Command

| Command | What it does |
|---|---|
| `amane new` | Asks where your shell should go (defaults to `~/.config/amane`), then creates `src/main.rs` there from a template, plus a `Cargo.toml` for your editor. Never overwrites an existing shell file. |
| `amane new <folder>` | Same, but inside `<folder>`. Amane remembers the folder, like `amane target`. |
| `amane new [folder] --example` | Creates a full example bar instead, split over several files, with workspace buttons for each monitor. Never overwrites an existing file. |
| `amane target <folder>` | Points Amane at a shell that already exists inside `<folder>`, for example after moving it or cloning your dotfiles. The folder needs a `src/main.rs`. |
| `amane target` | Prints the folder path Amane builds your shell from. |
| `amane dev` | Builds your shell, starts it, and rebuilds and restarts it on every save. |
| `amane compile` | Builds your shell and saves it as `~/.cache/amane/amane-shell`, without starting it. |
| `amane compile --restart` | Same as `amane compile`, then restarts the running shell with the new build, like `amane restart`. Restarts nothing if the build fails. |
| `amane run` | Starts the shell `amane compile` had saved in the background and runs the shell as a daemon. Never builds. Stops with an error msg if a shell is already running. |
| `amane restart` | Stops the shell `amane run` started, then starts the one `amane compile` had saved. Starts it anyway if none was running. Leaves the shell from `amane dev` alone. |
| `amane update [folder]` | Fetches your **amane clone**, **asks** then **pulls** it *(fast-forward only)* and reinstalls the command. Finds the clone from `AMANE_REPO`, or `~/amane`; a `folder` is remembered later. On a Nix installation it runs `nix profile upgrade amane` instead. |
| `amane clean` | Deletes the build output in `~/.cache/amane/project/`. Keeps the compiled shell, so `amane run` still works. Asks before deleting. |
| `amane clean --yes` | Same as `amane clean`, without asking first. |
| `amane ipc call <name> [arguments...]` | Calls a handler in the running shell. See [IPC](ipc.md). |

## Where things live

- **Your shell:** `~/.config/amane/src/`, or the `<folder>` you gave to `amane new` or `amane target`. `main.rs` is the entry point. You can add more files next to it and load them with `mod`, like in any other Rust program.
- **Which folder:** `amane target` prints it. A folder other than the default is saved in `~/.local/state/amane/config-path`, done per machine, so it stays out of your dotfiles. Run `amane target` again on each new machine.
- **Cargo.toml:** next to your `src/`. `amane new` writes it, and rust-analyzer reads it to find Amane in any editor. Every build rewrites the `amane` line in `[dependencies]` with this machine's path, so leave that line alone. Everything else is yours, see [Dependencies](#dependencies). If you keep your shell in git, that line changes per machine, so you may prefer to ignore `Cargo.toml`, `Cargo.lock` and `target/`.
- **Build files:** `~/.cache/amane/`.
  - `library/` holds the copy of Amane unpacked from the command.
  - `project/` holds the Cargo project generated around your `main.rs`.
  - `project/target/` holds the build output.
  - `amane-shell` is the shell `amane compile` saved, which `amane run` starts.

These paths follow `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` when they're set.

You never edit anything in the cache folder. Deleting it is safe. The next build makes it again from scratch.

`project/` is the big one, often close to **1 GB**. It only makes rebuilds fast, and your shell never reads it. Run `amane clean` to delete it. It asks first, because the next `amane compile` or `amane dev` then builds everything again from scratch, which takes a few minutes. To build and shrink in one go without the question:

```sh
amane compile && amane clean --yes
```

## Splitting your shell into files

`main.rs` can load other files the usual Rust way:

```text
~/.config/amane/src/
├── main.rs
├── bar.rs
└── clock.rs
```

```rust,ignore
// main.rs
mod bar;
mod clock;

use amane::App;

fn main() {
    App::new().window(bar::view).run();
}
```

`amane dev` watches the whole `src/` folder, plus `Cargo.toml` and `Cargo.lock`, so saving any of these files triggers a rebuild.

## How your shell is built

`amane dev` and `amane compile` share one build, so the build output is only kept on disk once.

| | Optimized |
|---|---|
| Your code | no, so a save rebuilds in seconds |
| Amane and its dependencies | yes |

Amane is always optimized, because unoptimized drawing is too slow for smooth animation. Your own code only describes the shell and Amane does the heavy work, so leaving your code unoptimized keeps rebuilds quick without slowing the shell down.

## Dependencies

Add crates under `[dependencies]` in the `Cargo.toml` next to your `src/`, like in any Rust project:

```toml
[dependencies]
chrono = "0.4"
```

The next `amane compile` or `amane dev` fetches and builds them. A relative `path = "..."` is read from your shell folder. After every good build, the resolved `Cargo.lock` is copied back next to your `Cargo.toml` when one exists there, so your versions stay pinned.

Most of what a shell needs is in the library already, so reach for a crate only when it isn't:

- system data, through [Built-in Services](system.md)
- running commands and reading their output, through [Commands and Files](processes.md)
- file watching, D-Bus, and IPC
