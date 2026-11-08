# The routine recipes come from the tools repository, in base.just: format,
# lint, test, and bump, for every language. This file adds the recipes of
# this repository alone.
import 'base.just'

# The crate links libpipewire, which exists only on Linux, so on macOS the
# Rust lint and tests fail in the build script of libspa-sys. A `ci` that
# failed there would block every commit on the Mac used for editing, so the
# macOS variant runs only the checks that work there: every format check, the
# Nix, TOML, YAML, and Markdown lints, and the Nix test. The Linux variant,
# and the CI workflows on Ubuntu, run the full set. Both replace the `ci` of
# base.just.
#
# Run every check: formatting, linters, and tests
[group('all')]
[linux]
ci: fmt-check lint test

# Run every check that works on macOS: all but the Rust lint and tests
[group('all')]
[macos]
ci: fmt-check lint-nix lint-toml lint-yaml lint-markdown test-nix

# Build the release binary, into target/release (not into $PATH)
[group('rust')]
build:
  {{ dev }} cargo build --release --locked

# `run` hands its extra arguments to the binary. just joins them with spaces
# and never quotes them, so no such argument may contain one.
#
# Run the binary, forwarding extra arguments to it
[group('rust')]
run *ARGS:
  {{ dev }} cargo run -- {{ ARGS }}
