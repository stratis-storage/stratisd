# Do test-loop last, since the tests for that target require sudo.
# Using sudo changes permissions on various directories. It is less trouble
# not to have to fix up permissions after every sudo'd test.
check: fmt build test docs test-loop

${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt --vers 0.6.3

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build

# Tests are in order of complexity, from least to greatest.
test-loop:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' cargo test -- --test test_force_flag_stratis
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' cargo test -- --test test_linear_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' cargo test -- --test test_thinpool_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' cargo test -- --test test_pool_blockdevs

test:
	RUSTFLAGS='-D warnings' \
	cargo test -- \
		--skip test_force_flag_stratis \
		--skip test_linear_device \
		--skip test_pool_blockdevs \
		--skip test_thinpool_device

docs:
	cargo doc --no-deps

.PHONY:
	check
	fmt
	build
	test
	docs
