{
  description = "Ambient LED backlight for gamescope, streamed to WLED";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # The shared configuration files, justfile recipes, and tools. `just
    # sync-tools` copies the files in at the version flake.lock pins.
    tools = {
      url = "github:larry0x/tools";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      tools,
    }:
    let
      # nixpkgs is the single source of truth for the Rust toolchain: the
      # devShell and CI both take it from here. The crate links libpipewire, so
      # it only builds on Linux; the devShell still evaluates on the Mac used
      # for editing, where it provides the formatters (see shell.nix).
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];

      forEachSystem = f: nixpkgs.lib.genAttrs systems f;

      # The files this repository takes from the tools repository. This
      # repository has recipes of its own, so the base justfile is base.just,
      # which the justfile imports.
      toolsFiles = [
        ".editorconfig"
        ".gitignore"
        ".markdownlint-cli2.yaml"
        ".yamlfmt.yaml"
        ".yamllint.yaml"
        "clippy.toml"
        "rustfmt.toml"
        "taplo.toml"
        {
          from = "justfile";
          to = "base.just";
        }
      ];
    in
    {
      # Fails while a file taken from the tools repository differs from the
      # pinned version, or a workflow calls the tools repository at another
      # commit.
      checks = forEachSystem (system: {
        tools = tools.lib.check {
          pkgs = nixpkgs.legacyPackages.${system};
          src = self;
          files = toolsFiles;
        };
      });

      # `just sync-tools` runs this, to copy the files in and pin the calls.
      apps = forEachSystem (system: {
        sync-tools = tools.lib.sync {
          pkgs = nixpkgs.legacyPackages.${system};
          files = toolsFiles;
        };
      });

      devShells = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.callPackage ./shell.nix {
            toolsPackages = tools.lib.packages pkgs;
          };
        }
      );
    };
}
