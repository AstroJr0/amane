# Installation

Amane comes as one command, `amane`. The command carries a full copy of the library inside it. It builds your shell from your config folder, so you never set up a Cargo project yourself.

## NixOS

The repository is a flake. Its package wraps `amane` with everything it needs at build time (cargo, rustc, pkg-config, and the system libraries), so there's nothing else to install.

To try it without installing:

```sh
nix run github:MystiaFin/amane -- new
```

To install it for your user:

```sh
nix profile install github:MystiaFin/amane
```

To install it system-wide, add it to your flake inputs:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    amane = {
      url = "github:MystiaFin/amane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, amane, ... }: {
    nixosConfigurations.your-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        {
          environment.systemPackages = [ amane.packages.x86_64-linux.default ];
        }
      ];
    };
  };
}
```

Replace `your-host` with your host's name, then rebuild.

The `follows` line builds Amane with your system's nixpkgs, so it uses the same glibc as your graphics driver. Without it, the shell can fail to find your GPU.

## Other distributions

You need a Rust toolchain, `pkg-config`, and these libraries with their development headers:

| Library | Debian / Ubuntu | Arch |
|---|---|---|
| Wayland | `libwayland-dev` | `wayland` |
| xkbcommon | `libxkbcommon-dev` | `libxkbcommon` |
| fontconfig | `libfontconfig1-dev` | `fontconfig` |
| Vulkan loader | `libvulkan1` | `vulkan-icd-loader` |
| PulseAudio client | `libpulse-dev` | `libpulse` |
| PAM | `libpam0g-dev` | `pam` |

Then build and install the command:

```sh
git clone https://github.com/MystiaFin/amane
cd amane
cargo install --path cli
```

`cargo install` puts `amane` in `~/.cargo/bin`. Make sure that folder is on your `PATH`.

The `amane` command runs `cargo` every time it builds your shell, so keep the Rust toolchain installed.

## Updating

The library is copied into the `amane` command when built. You can update your setup seamlessly using the built-in command:

```sh
amane update
```

>[!CAUTION]
> `amane update` does work on NixOS, but it executes `nix profile upgrade amane` in the background.
> If amane was installed System-Wide via Nix Flakes, DO NOT confirm the confirmation prompt
> as packages must be updated through your flake configuration instead.

For manual updates:
- NixOS (Profile): `nix profile upgrade amane`
- Other Distributions: `git pull` in the clone, then run `cargo install --path cli` again.

The next `amane compile` or `amane dev` will automatically rebuild your shell with the new library and features!


