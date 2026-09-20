{
  description = "Ambient LED backlight for gamescope, streamed to WLED";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { nixpkgs, ... }:
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
    in
    {
      devShells = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.callPackage ./shell.nix { };
        }
      );
    };
}
