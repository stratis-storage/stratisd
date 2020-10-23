ifeq ($(origin TARGET), undefined)
else
  TARGET_ARGS = --target=${TARGET}
endif

DESTDIR ?=
PREFIX ?= /usr
LIBEXECDIR ?= $(PREFIX)/libexec
DATADIR ?= $(PREFIX)/share
UDEVDIR ?= $(PREFIX)/lib/udev
MANDIR ?= $(DATADIR)/man
UNITDIR ?= $(PREFIX)/lib/systemd/system

RELEASE_VERSION ?= 9.9.9

RUST_2018_IDIOMS = -D bare-trait-objects \
                   -D ellipsis-inclusive-range-patterns

DENY = -D warnings -D future-incompatible -D unused ${RUST_2018_IDIOMS}

# Clippy-related lints
CLIPPY_CARGO = -D clippy::cargo_common_metadata \
               -D clippy::wildcard_dependencies

# Explicitly allow these lints because they don't seem helpful
# doc_markdown: we would rather have useful than well-formatted docs
# map_err_ignore: we generally drop the errors for a reason
# option_if_let_else: causing problems with if-else chains
# similar_names: judges "yes" and "res" to be too similar
CLIPPY_PEDANTIC_USELESS = -A clippy::doc_markdown \
                          -A clippy::map_err_ignore \
                          -A clippy::option_if_let_else \
                          -A clippy::similar_names

# Clippy allow/deny adjudications for pedantic lints
#
# Allows represent lints we fail but which we may
# conclude are helpful at some time.
CLIPPY_PEDANTIC = -D clippy::await_holding_lock \
                  -D clippy::await_holding_refcell_ref \
                  -D clippy::cast_lossless \
                  -D clippy::cast_possible_truncation \
                  -A clippy::cast_possible_wrap \
                  -D clippy::cast_precision_loss \
                  -D clippy::cast_ptr_alignment \
                  -A clippy::cast_sign_loss \
                  -D clippy::checked_conversions \
                  -D clippy::copy_iterator \
                  -A clippy::default_trait_access \
                  -D clippy::empty_enum \
                  -D clippy::enum_glob_use \
                  -D clippy::expl_impl_clone_on_copy \
                  -D clippy::explicit_deref_methods \
                  -D clippy::explicit_into_iter_loop \
                  -A clippy::explicit_iter_loop \
                  -A clippy::filter_map \
                  -A clippy::filter_map_next \
                  -D clippy::fn_params_excessive_bools \
                  -A clippy::if_not_else \
                  -D clippy::implicit_hasher \
                  -D clippy::implicit_saturating_sub \
                  -D clippy::inefficient_to_string \
                  -D clippy::inline_always \
                  -D clippy::invalid_upcast_comparisons \
                  -A clippy::items_after_statements \
                  -D clippy::large_digit_groups \
                  -D clippy::large_stack_arrays \
                  -D clippy::large_types_passed_by_value \
                  -D clippy::let_unit_value \
                  -D clippy::linkedlist \
                  -D clippy::macro_use_imports \
                  -D clippy::manual_ok_or \
                  -D clippy::map_flatten \
                  -A clippy::map_unwrap_or \
                  -D clippy::match_bool \
                  -D clippy::match_on_vec_items \
                  -A clippy::match_same_arms \
                  -D clippy::match_wild_err_arm \
                  -A clippy::match_wildcard_for_single_variants \
                  -D clippy::maybe_infinite_iter \
                  -A clippy::missing_errors_doc \
                  -A clippy::module_name_repetitions \
                  -A clippy::must_use_candidate \
                  -D clippy::mut_mut \
                  -D clippy::needless_continue \
                  -A clippy::needless_pass_by_value \
                  -A clippy::non_ascii_literal \
                  -A clippy::option_if_let_else \
                  -D clippy::option_option \
                  -D clippy::pub_enum_variant_names \
                  -D clippy::range_minus_one \
                  -D clippy::range_plus_one \
                  -A clippy::redundant_closure_for_method_calls \
                  -D clippy::ref_option_ref \
                  -D clippy::same_functions_in_if_condition \
                  -A clippy::shadow_unrelated \
                  -A clippy::single_match_else \
                  -D clippy::string_add_assign \
                  -D clippy::struct_excessive_bools \
                  -A clippy::too_many_lines \
                  -D clippy::trait_duplication_in_bounds \
                  -D clippy::trivially_copy_pass_by_ref \
                  -D clippy::type_repetition_in_bounds \
                  -D clippy::unicode_not_nfc \
                  -D clippy::unnested_or_patterns \
                  -D clippy::unreadable_literal \
                  -D clippy::unsafe_derive_deserialize \
                  -A clippy::unseparated_literal_suffix \
                  -A clippy::unused_self \
                  -D clippy::used_underscore_binding \
                  -D clippy::used_underscore_binding \
                  -D clippy::verbose_bit_mask \
                  -D clippy::wildcard_imports

${HOME}/.cargo/bin/cargo-outdated:
	cargo install cargo-outdated

${HOME}/.cargo/bin/cargo-license:
	cargo install cargo-license

${HOME}/.cargo/bin/cargo-bloat:
	cargo install cargo-bloat

${HOME}/.cargo/bin/cargo-audit:
	cargo install cargo-audit

outdated: ${HOME}/.cargo/bin/cargo-outdated
	PATH=${HOME}/.cargo/bin:${PATH} cargo outdated

license: ${HOME}/.cargo/bin/cargo-license
	PATH=${HOME}/.cargo/bin:${PATH} cargo license

bloat: ${HOME}/.cargo/bin/cargo-bloat
	PATH=${HOME}/.cargo/bin:${PATH} cargo bloat --release
	PATH=${HOME}/.cargo/bin:${PATH} cargo bloat --release --crates

audit: ${HOME}/.cargo/bin/cargo-audit
	PATH=${HOME}/.cargo/bin:${PATH} cargo audit -D warnings

${PWD}/stratisd-vendor.tar.gz:
	cargo vendor
	tar -czvf stratisd-vendor.tar.gz vendor

create-release: ${PWD}/stratisd-vendor.tar.gz
	mv ${PWD}/stratisd-vendor.tar.gz ${PWD}/stratisd-${RELEASE_VERSION}-vendor.tar.gz
	${PWD}/code_maintenance/create_release.py ${RELEASE_VERSION}
	rm -rf vendor
	rm stratisd-${RELEASE_VERSION}-vendor.tar.gz

fmt:
	cargo fmt

fmt-travis:
	cargo fmt -- --check

build:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${TARGET_ARGS}

build-tests:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo test --no-run ${TARGET_ARGS}

build-no-default:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --no-default-features ${TARGET_ARGS}

build-extras:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --features extras ${TARGET_ARGS}

build-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis-min --bin=stratisd-min --no-default-features \
	--features min,systemd_notify ${TARGET_ARGS}

stratis-dumpmetadata:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis_dumpmetadata --features extras ${TARGET_ARGS}

stratis-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis-min --features min ${TARGET_ARGS}

stratisd-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratisd-min --features min ${TARGET_ARGS}

profiledir := $(shell if test -d target/release; then echo target/release; else echo target/debug; fi)
install: release docs
	install -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) $(profiledir)/stratisd
	install -Dpm0644 -t $(DESTDIR)$(DATADIR)/dbus-1/system.d stratisd.conf
	install -Dpm0644 -t $(DESTDIR)$(MANDIR)/man8 docs/stratisd.8
	install -Dpm0644 -t $(DESTDIR)$(UDEVDIR)/rules.d udev/14-stratisd.rules
	install -Dpm0644 -t $(DESTDIR)$(UNITDIR) stratisd.service
	install -Dpm0755 -t $(DESTDIR)$(PREFIX)/bin developer_tools/stratis_migrate_symlinks.sh

release:
	RUSTFLAGS="${DENY}" cargo build --release

test-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test loop_

test-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test real_

test:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 cargo test -- --skip real_ --skip loop_

yamllint:
	yamllint --strict .github/workflows/main.yml

docs: stratisd.8 docs-rust

docs-travis: docs-rust

docs-rust:
	cargo doc --no-deps

stratisd.8: docs/stratisd.txt
	a2x -f manpage docs/stratisd.txt

stratisd.8.gz: stratisd.8
	gzip --stdout docs/stratisd.8 > docs/stratisd.8.gz

clippy:
	RUSTFLAGS="${DENY}" cargo clippy --all-targets --all-features -- ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS} ${CLIPPY_CARGO}

.PHONY:
	audit
	bloat
	build
	clippy
	create-release
	docs
	docs-rust
	docs-travis
	fmt
	fmt-travis
	install
	license
	outdated
	release
	build-min
	test
	test-loop
	test-real
	yamllint
