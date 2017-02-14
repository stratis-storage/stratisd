check: fmt build test docs

${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt --vers 0.6.3

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build

test:
	cargo test -- --skip test_pools --skip test_blockdev_setup \
		--skip test_lineardev_setup --skip test_thinpoolsetup_setup

docs:
	cargo doc --no-deps

.PHONY:
	check
	fmt
	build
	test
	docs
