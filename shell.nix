# The development shell: every tool the just recipes wrap, all from nixpkgs,
# which flake.nix pins.
#
# The crate links libpipewire, which exists only on Linux, so its build inputs
# are added only there. On the Mac used for editing this shell still provides
# the formatters and the Nix tools, so `just fmt` and the Nix checks run there;
# the Rust build and tests run on Linux.
{
  mkShell,
  lib,
  stdenv,
  cargo,
  clippy,
  rustc,
  rustfmt,
  taplo,
  just,
  nixfmt,
  statix,
  deadnix,
  pkg-config,
  rustPlatform,
  pipewire,
}:
mkShell {
  nativeBuildInputs = [
    # The task runner. Every recipe wraps its commands in `nix develop`, and
    # CI runs the same recipes, so this is the entry point on both sides.
    just

    # The Rust toolchain: four packages built from one rustc release, so their
    # versions cannot skew; bump with `just bump-toolchain`. cargo does not
    # pull in the compiler by itself, so rustc must be listed.
    cargo
    clippy
    rustc
    rustfmt

    # Formatter and linter for TOML files, configured by taplo.toml.
    taplo

    # Formatter and linters for the Nix files.
    nixfmt
    statix
    deadnix
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [
    # Building the pipewire crate: pkg-config finds libpipewire, and the
    # bindgen hook provides libclang for its generated bindings.
    pkg-config
    rustPlatform.bindgenHook
  ];

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    # libpipewire itself, found through pkg-config above.
    pipewire
  ];
}
