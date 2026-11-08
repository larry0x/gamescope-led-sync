# The development shell: every tool the just recipes call, all from nixpkgs,
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
  pkg-config,
  rustPlatform,
  pipewire,

  # just, and the formatters and linters for Nix, TOML, YAML, and Markdown,
  # from the tools repository, at the versions of the nixpkgs pinned here.
  toolsPackages,
}:
mkShell {
  nativeBuildInputs =
    toolsPackages
    ++ [
      # The Rust toolchain: four packages built from one rustc release, so
      # their versions cannot skew; bump with `just bump-tools`. cargo does not
      # pull in the compiler by itself, so rustc must be listed.
      cargo
      clippy
      rustc
      rustfmt
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
