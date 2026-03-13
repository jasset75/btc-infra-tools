set shell := ["zsh", "-cu"]

default:
  @just --list

build:
  cargo build --workspace

install:
  cargo install --path crates/infractl-cli --locked --root ~/.local --force

check:
  cargo check --workspace

clippy:
  cargo clippy --workspace --all-targets --all-features -- -D warnings

clippy-fix:
  cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged

test:
  cargo test --all-targets
