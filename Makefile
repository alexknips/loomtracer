.PHONY: build test install fmt clippy clean

build:
	cargo build

test:
	cargo test

# Builds the `loomtracer` CLI from this checkout and installs it to
# ~/.local/bin/loomtracer — NOT cargo's default install root (~/.cargo/bin).
# On Gas Town rigs ~/.local/bin comes first on $PATH (it's also where
# scripts/install.sh puts published releases), so installing to
# ~/.cargo/bin instead is silently shadowed: `make install` "succeeds" but
# `loomtracer` on PATH keeps resolving whatever stale binary is already in
# ~/.local/bin. This is exactly how the fleet ran a two-week-old build
# while master kept moving (hivemind-zdsh.7). Run this after every
# rebase/pull so agent tooling (MCP config, capture scripts, plugins) that
# resolves `loomtracer` on PATH always finds the current version — confirm
# with `loomtracer --version`, which now reports <semver>+<build sha>.
install:
	cargo install --path . --locked --bin loomtracer --root "$(HOME)/.local"

fmt:
	cargo fmt

clippy:
	cargo clippy --all-targets -- -D warnings

clean:
	cargo clean
