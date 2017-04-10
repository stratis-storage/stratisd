# Do test-loop last, since the tests for that target require sudo.
# Using sudo changes permissions on various directories. It is less trouble
# not to have to fix up permissions after every sudo'd test.
check: fmt build docs test test-loop

${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt --vers 0.6.3

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build

# Tests are in order of complexity, from least to greatest.
test-loop:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_force_flag_dirty
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_force_flag_stratis
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_variable_length_metadata_times
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_linear_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_thinpool_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_pool_blockdevs


# Tests are in order of complexity, from least to greatest.
test-real:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_force_flag_dirty
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_force_flag_stratis
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_variable_length_metadata_times
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_linear_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_thinpool_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_pool_blockdevs

test:
	RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --skip real_ --skip loop_

docs:
	cargo doc --no-deps

.PHONY:
	check
	fmt
	build
	test
	docs
