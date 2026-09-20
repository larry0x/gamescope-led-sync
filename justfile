# Every tool comes from the nix devShell, so a clean checkout needs only nix.
dev := "nix develop --command"

# List available recipes
default:
  @just --list

# ------------------------------------ All -------------------------------------

# Format every source: Rust, TOML, and Nix
[group('all')]
fmt: fmt-rust fmt-nix

# Report formatting drift in every source, changing nothing
[group('all')]
fmt-check: fmt-check-rust fmt-check-nix

# Lint every source
[group('all')]
lint: lint-rust lint-nix

# Run every test suite
[group('all')]
test: test-rust

# ------------------------------------ Rust ------------------------------------

# Format the Rust sources (TOML + Rust)
[group('rust')]
fmt-rust:
  {{ dev }} taplo fmt
  {{ dev }} cargo fmt --all

# Report Rust formatting drift, changing nothing
[group('rust')]
fmt-check-rust:
  {{ dev }} taplo fmt --check --diff
  {{ dev }} cargo fmt --all --check

# Lint the Rust sources
[group('rust')]
lint-rust:
  {{ dev }} taplo lint
  {{ dev }} cargo clippy --bins --tests --benches --examples --all-features --all-targets -- -D warnings

# Run the Rust tests
[group('rust')]
test-rust:
  RUST_BACKTRACE=1 {{ dev }} cargo test --all-features --tests -- --nocapture

# Build the release binary, into target/release (not into $PATH)
[group('rust')]
build:
  {{ dev }} cargo build --release --locked

# `run` hands its extra arguments to the binary. just joins them with spaces
# and never quotes them, so no such argument may contain one.

# Run the binary, forwarding extra arguments to it
[group('rust')]
run *ARGS:
  {{ dev }} cargo run -- {{ ARGS }}

# ------------------------------------ Nix -------------------------------------

# Format the Nix files
[group('nix')]
fmt-nix:
  {{ dev }} nixfmt $(git ls-files '*.nix')

# Report Nix formatting drift, changing nothing
[group('nix')]
fmt-check-nix:
  {{ dev }} nixfmt --check $(git ls-files '*.nix')

# The last line is nix itself, not a tool out of the shell, which is why it
# carries no prefix. Development happens on a Mac, so x86_64-linux is never
# built there. This evaluates every output for both systems and builds none of
# them, which is the cheap way to catch a Linux-only mistake.

# Lint the Nix files
[group('nix')]
lint-nix:
  {{ dev }} statix check .
  {{ dev }} deadnix --fail .
  nix flake check --all-systems

# ------------------------------------ Bump ------------------------------------

# Bump the pinned toolchain (nixpkgs)
[group('bump')]
bump-toolchain:
  nix flake update nixpkgs

# Bump the versions of cargo dependencies
[group('bump')]
bump-deps:
  {{ dev }} cargo update
