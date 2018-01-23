${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt --vers 0.8.3

${HOME}/.cargo/bin/cargo-tree:
	cargo install cargo-tree

${HOME}/.cargo/bin/cargo-outdated:
	cargo install cargo-outdated

tree: ${HOME}/.cargo/bin/cargo-tree
	PATH=${HOME}/.cargo/bin:${PATH} cargo tree

outdated: ${HOME}/.cargo/bin/cargo-outdated
	PATH=${HOME}/.cargo/bin:${PATH} cargo outdated

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt

travis_fmt:
	rustup run stable cargo install rustfmt --vers 0.8.3 --force
	cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build --features "dbus_enabled"

test-loop:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test loop_

test-real:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test real_

test-travis:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test travis_

test:
	RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --skip real_ --skip loop_ --skip travis_

docs:
	cargo doc --no-deps

clippy:
	RUSTFLAGS='-D warnings' cargo build --features "clippy"

.PHONY:
	build
	clippy
	docs
	fmt
	outdated
	test
	test-real
	test-loop
	tree
