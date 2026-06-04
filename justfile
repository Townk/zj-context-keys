# zj-context-keys task runner.
#
# Run `just --list` to see all recipes.

wasm_path := justfile_directory() + "/target/wasm32-wasip1/release/zj-context-keys.wasm"
plugin_dir := env_var('HOME') + "/.config/zellij/plugins"

# Build the plugin in release mode for Zellij's wasm target.
build:
    cargo build --release --target wasm32-wasip1

# Run the host-side unit tests.
test:
    cargo test

# Check formatting and lint with the same strictness as CI.
lint:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings

# Format the source tree.
fmt:
    cargo fmt

# Build, then install the wasm into your local Zellij plugins directory.
install: build
    mkdir -p "{{ plugin_dir }}"
    cp "{{ wasm_path }}" "{{ plugin_dir }}/"
