# Every tool comes from the nix devShell, so a clean checkout needs only nix.
dev := "nix develop --command"

# The Nix sources: tracked files plus new ones git does not ignore, minus
# tracked files deleted from disk but not yet staged. So a new file is
# covered before its first `git add`, a git-ignored scratch copy is not, and
# a pending deletion does not hand a tool a path that is gone. A path may not
# contain a space.
nix := "$(git ls-files --cached --others --exclude-standard --deduplicate '*.nix' | while IFS= read -r f; do [ -f \"$f\" ] && echo \"$f\"; done)"

# The global pre-commit hook on pc1 runs `just ci` before every commit, and
# aborts the commit if it fails.

# List available recipes
default:
  @just --list

# ------------------------------------ All -------------------------------------

# The crate links libpipewire, which exists only on Linux, so on macOS the
# Rust lint and tests fail in the build script of libspa-sys. A `ci` that
# failed there would block every commit on the Mac used for editing, so the
# macOS variant runs only the checks that work there: every format check, the
# TOML lint, the Nix lints, and the Markdown lint. The Linux variant, and the
# CI workflows on Ubuntu, run the full set.

# Run every check: formatting, linters, and tests
[group('all')]
[linux]
ci: fmt-check lint test

# Run every check that works on macOS: formatting, TOML, Nix, and Markdown
[group('all')]
[macos]
ci: fmt-check lint-nix lint-markdown
  {{ dev }} taplo lint

# Format every source: Rust, TOML, Nix, and Markdown
[group('all')]
fmt: fmt-rust fmt-nix fmt-markdown

# Report formatting drift in every source, changing nothing
[group('all')]
fmt-check: fmt-check-rust fmt-check-nix

# Lint every source
[group('all')]
lint: lint-rust lint-nix lint-markdown

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
  {{ dev }} nixfmt {{ nix }}

# Report Nix formatting drift, changing nothing
[group('nix')]
fmt-check-nix:
  {{ dev }} nixfmt --check {{ nix }}

# The last line is nix itself, not a tool out of the shell, which is why it
# carries no prefix. Development happens on a Mac, so x86_64-linux is never
# built there. This evaluates every output for both systems and builds none of
# them, which is the cheap way to catch a Linux-only mistake.

# statix takes one path per run and does not honor every ignore source, such
# as .git/info/exclude, so lint-nix loops over the list.

# The third line runs nixf-tidy: the checks of nixf, the library behind
# nixd, the language server that VS Code shows, so whatever the editor flags
# in a Nix file fails here too. --variable-lookup adds the undefined
# variable check that the editor runs as well. nixf-tidy reports a JSON list
# on stdout, empty for a clean file, and exits 0 either way, a parse error
# included, so the line fails on any non-empty list. nixf_report is the jq
# filter that prints each finding as file:line:column: message (code),
# filling the {} slots in the message from its args. It is a variable,
# passed to the shell as an argument, so that its quotes need no escaping
# inside the shell's own quoted script.
nixf_report := '.[] | "\($f):\(.range.lCur.line + 1):\(.range.lCur.column + 1): \(reduce .args[] as $a (.message; sub("\\{\\}"; $a))) (\(.sname))"'

# Lint the Nix files
[group('nix')]
lint-nix:
  {{ dev }} sh -c 's=0; for f in "$@"; do statix check "$f" || s=1; done; exit $s' _ {{ nix }}
  {{ dev }} deadnix --fail {{ nix }}
  {{ dev }} sh -c 'q=$1; shift; s=0; for f in "$@"; do \
    out=$(nixf-tidy --variable-lookup < "$f") || s=1; \
    [ "$out" = "[]" ] || { s=1; printf "%s\n" "$out" | jq -r --arg f "$f" "$q"; }; \
  done; exit $s' _ '{{ nixf_report }}' {{ nix }}
  nix flake check --all-systems

# ---------------------------------- Markdown ----------------------------------
#
# markdownlint-cli2 reads the files to lint, and the rules, from
# .markdownlint-cli2.yaml. --fix repairs what it can, and reports the rest.

# Fix Markdown lint errors that can be fixed automatically
[group('markdown')]
fmt-markdown:
  {{ dev }} markdownlint-cli2 --fix

# Lint the Markdown files
[group('markdown')]
lint-markdown:
  {{ dev }} markdownlint-cli2

# ------------------------------------ Bump ------------------------------------

# Bump the pinned toolchain (nixpkgs)
[group('bump')]
bump-toolchain:
  nix flake update nixpkgs

# Bump the versions of cargo dependencies
[group('bump')]
bump-deps:
  {{ dev }} cargo update
