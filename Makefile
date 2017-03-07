check: fmt build test docs

${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build

test:
	RUSTFLAGS='-D warnings' \
	cargo test -- --skip test_pools --skip test_blockdev_setup \
		--skip test_lineardev_setup --skip test_thinpool

docs:
	cargo doc --no-deps

.PHONY:
	check
	fmt
	build
	test
	docs
