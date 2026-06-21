# zj-context-keys task runner.
#
# Run `just --list` to see all recipes.

wasm_path := justfile_directory() + "/target/wasm32-wasip1/release/zj-context-keys.wasm"
plugin_dir := env_var('HOME') + "/.config/zellij/plugins"
repo := "Townk/zj-context-keys"

# Ensure the wasm target Zellij loads is installed in the active toolchain.
# `rust-toolchain.toml` pins `wasm32-wasip1`, but rustup doesn't always
# auto-install targets from that file, so a plain `just build` can fail with
# `can't find crate for core` until the target is added once.
target:
    rustup target list --installed | grep -q '^wasm32-wasip1$' || rustup target add wasm32-wasip1

# Build the plugin in release mode for Zellij's wasm target.
build: target
    cargo build --release --target wasm32-wasip1

# Run the host-side unit tests.
test:
    cargo test

# Check formatting and lint with the same strictness as CI.
lint: target
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings

# Format the source tree.
fmt:
    cargo fmt

# Build, then install the wasm into your local Zellij plugins directory.
install: build
    mkdir -p "{{ plugin_dir }}"
    cp "{{ wasm_path }}" "{{ plugin_dir }}/"

# Bump version, run the quality gate, commit, tag & push: `just release 0.2.0`.
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ version }}"
    tag="v$ver"

    if [[ ! "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "error: version must be X.Y.Z (got '$ver')" >&2
        exit 1
    fi

    branch="$(git rev-parse --abbrev-ref HEAD)"
    if [[ "$branch" != "master" ]]; then
        echo "error: releases are cut from 'master' (currently on '$branch')" >&2
        exit 1
    fi

    if [[ -n "$(git status --porcelain)" ]]; then
        echo "error: working tree is dirty; commit or stash changes first" >&2
        exit 1
    fi

    git fetch --quiet origin master --tags
    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
        echo "error: tag $tag already exists" >&2
        exit 1
    fi
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/master)" ]]; then
        echo "error: local master is not in sync with origin/master; pull/push first" >&2
        exit 1
    fi

    # Ensure the wasm target is installed before the quality gate needs it.
    rustup target list --installed | grep -q '^wasm32-wasip1$' || rustup target add wasm32-wasip1

    # Quality gate before mutating anything.
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

    # Bump the version in Cargo.toml and the pinned URL example in the README.
    sed -i.bak -E "s/^version = \".*\"/version = \"$ver\"/" Cargo.toml
    sed -i.bak -E "s#releases/download/v[0-9]+\.[0-9]+\.[0-9]+/#releases/download/$tag/#g" README.md
    rm -f Cargo.toml.bak README.md.bak

    # Refresh Cargo.lock and confirm the release artifact still builds.
    cargo build --release --target wasm32-wasip1

    git add Cargo.toml Cargo.lock README.md
    git commit -m "chore(release): $tag"
    git tag -a "$tag" -m "$tag"
    git push origin master "$tag"

    echo "Released $tag -> https://github.com/{{ repo }}/releases/tag/$tag"
