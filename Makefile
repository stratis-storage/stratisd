${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt --vers 0.8.3

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff

build:
	RUSTFLAGS='-D warnings' cargo build

test-loop:
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_empty_pool
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_force_flag_dirty
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_force_flag_stratis
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_variable_length_metadata_times
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_linear_device
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_thinpool_device
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_pool_blockdevs
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_initialize
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_basic_metadata
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_setup
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_thinpool_expand
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_thinpool_thindev_destroy
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_pool_rename
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_blockdevmgr_used
#	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test loop_test_filesystem_rename
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_LOG='libstratis=debug,devicemapper=debug' RUST_BACKTRACE=1 cargo test -- --nocapture --test loop_test_pool_setup

test-real:
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_empty_pool
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_force_flag_dirty
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_force_flag_stratis
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_variable_length_metadata_times
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_linear_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_thinpool_device
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_pool_blockdevs
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_initialize
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_basic_metadata
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_setup
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_thinpool_expand
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_thinpool_thindev_destroy
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_pool_rename
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_blockdevmgr_used
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_filesystem_rename
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 which thin_check
	sudo env "PATH=${PATH}" RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --test real_test_pool_setup

test:
	RUSTFLAGS='-D warnings' RUST_BACKTRACE=1 cargo test -- --skip real_ --skip loop_

docs:
	cargo doc --no-deps

clippy:
	RUSTFLAGS='-D warnings' cargo build --features "clippy"

.PHONY:
	fmt
	build
	test
	docs
	test-real
	test-loop
	clippy
