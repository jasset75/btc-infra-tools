set shell := ["zsh", "-cu"]

default:
  @just --list

build:
  cargo build --workspace

install:
  cargo install --path crates/infractl-cli --locked --root ~/.local --force
  ~/.local/bin/belter --version

install-latest-stable:
  #!/usr/bin/env zsh
  set -eu
  git fetch --tags origin
  tag="$(git tag --list 'v*' --sort=-v:refname | head -n 1)"
  if [[ -z "${tag}" ]]; then
    echo "No release tags found after fetching from origin." >&2
    exit 1
  fi
  git checkout "${tag}"
  cargo install --path crates/infractl-cli --locked --root ~/.local --force
  ~/.local/bin/belter --version

check:
  cargo check --workspace

clippy:
  cargo clippy --workspace --all-targets --all-features -- -D warnings

clippy-fix:
  cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged

test:
  cargo test --workspace

test-cli:
  cargo test -p belter --all-targets
