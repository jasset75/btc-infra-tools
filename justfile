set shell := ["zsh", "-cu"]

default:
  @just --list

build:
  cargo build --workspace

_prepare-install-config:
  #!/usr/bin/env zsh
  set -eu
  config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
  belter_config_dir="${config_home}/belter"
  mkdir -p "${belter_config_dir}"
  if [[ ! -f "${belter_config_dir}/.env" ]]; then
    if [[ -f .env ]]; then
      cp .env "${belter_config_dir}/.env"
    else
      cp .env.example "${belter_config_dir}/.env"
    fi
  fi
  if [[ ! -f "${belter_config_dir}/belter.toml" ]]; then
    cp belter.toml "${belter_config_dir}/belter.toml"
  fi

install:
  just _prepare-install-config
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
  just _prepare-install-config
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
