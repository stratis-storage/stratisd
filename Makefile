check: fmt build test docs

${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt --vers 0.6.3

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build

test-loop:
	RUSTFLAGS='-D warnings' cargo test -- --test test_force_flag
	RUSTFLAGS='-D warnings' cargo test -- --test test_new_blockdevs
	RUSTFLAGS='-D warnings' cargo test -- --test test_setup

test:
	RUSTFLAGS='-D warnings' \
	cargo test -- --skip test_pools \
		--skip test_lineardev_setup --skip test_thinpool \
		--skip test_force_flag --skip test_new_blockdevs \
		--skip test_setup

docs:
	cargo doc --no-deps

.PHONY:
	check
	fmt
	build
	test
	docs
