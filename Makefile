TARGET ?= "x86_64-unknown-linux-gnu"

${HOME}/.cargo/bin/cargo-tree:
	cargo install cargo-tree

${HOME}/.cargo/bin/cargo-outdated:
	cargo install cargo-outdated

${HOME}/.cargo/bin/cargo-script:
	cargo install cargo-script

tree: ${HOME}/.cargo/bin/cargo-tree
	PATH=${HOME}/.cargo/bin:${PATH} cargo tree

outdated: ${HOME}/.cargo/bin/cargo-outdated
	PATH=${HOME}/.cargo/bin:${PATH} cargo outdated

fmt:
	cargo fmt

fmt-travis:
	rustup default 1.27.0
	rustup component add rustfmt-preview
	cargo fmt -- --write-mode=check

build:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS='-D warnings' \
	cargo build --target $(TARGET)

build-no-default:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS='-D warnings' \
	cargo build --no-default-features --target $(TARGET)

test-loop:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test loop_

test-real:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test real_bogus_

test-travis:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test travis_

test:
	RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --skip real_ --skip loop_ --skip travis_

docs: stratisd.8 docs-rust

docs-travis: docs-rust

docs-rust:
	cargo doc --no-deps

stratisd.8: docs/stratisd.txt
	a2x -f manpage docs/stratisd.txt

stratisd.8.gz: stratisd.8
	gzip --stdout docs/stratisd.8 > docs/stratisd.8.gz

clippy:
	RUSTFLAGS='-D warnings' cargo clippy

uml-graphs: ${HOME}/.cargo/bin/cargo-script
	PATH=${HOME}/.cargo/bin:${PATH} cargo script scripts/uml_graphs.rs

.PHONY:
	build
	clippy
	docs
	docs-rust
	docs-travis
	fmt
	fmt-travis
	outdated
	test
	test-loop
	test-real
	test-travis
	tree
	uml-graphs
