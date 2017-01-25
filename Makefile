check: fmt build test

${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt --vers 0.6.3

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build

test:
	cargo test

.PHONY:
	check
	fmt
	build
	test
