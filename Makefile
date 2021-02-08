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

RUST_2018_IDIOMS = -D bare-trait-objects \
                   -D ellipsis-inclusive-range-patterns

DENY = -D warnings -D future-incompatible -D unused ${RUST_2018_IDIOMS}

# Clippy deny variable, including allows for troublesome lints.
# Notable allows:
# map_err_ignore: we generally drop the errors for a reason
# option_if_let_else: causing problems with if-else chains
# similar_names: judges "yes" and "res" to be too similar
CLIPPY_DENY = -D clippy::pedantic \
              -A clippy::cast_possible_wrap \
              -A clippy::cast_sign_loss \
              -A clippy::default_trait_access \
              -A clippy::doc_markdown \
              -A clippy::explicit_iter_loop \
              -A clippy::filter_map \
              -A clippy::filter_map_next \
              -A clippy::find_map \
              -A clippy::if_not_else \
              -A clippy::items_after_statements \
              -A clippy::map_err_ignore \
              -A clippy::map_unwrap_or \
              -A clippy::match_same_arms \
              -A clippy::match_wildcard_for_single_variants \
              -A clippy::missing_errors_doc \
              -A clippy::must_use_candidate \
              -A clippy::module_name_repetitions \
              -A clippy::needless_pass_by_value \
              -A clippy::non_ascii_literal \
              -A clippy::option_if_let_else \
              -A clippy::redundant-closure-for-method-calls \
              -A clippy::shadow_unrelated \
              -A clippy::similar_names \
              -A clippy::single_match_else \
              -A clippy::too_many_lines \
              -A clippy::unseparated_literal_suffix \
              -A clippy::unused_self

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
	${PWD}/code_maintenance/create_release.py ${RELEASE_VERSION}
	rm -rf vendor
	rm stratisd-vendor.tar.gz

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

stratis-dumpmetadata:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis_dumpmetadata --features extras ${TARGET_ARGS}

stratis-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis-min --features extras ${TARGET_ARGS}

profiledir := $(shell if test -d target/release; then echo target/release; else echo target/debug; fi)
install: build docs
	install -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) $(profiledir)/stratisd
	install -Dpm0755 -t $(DESTDIR)$(UDEVDIR) $(profiledir)/stratis_uuids_to_names
	install -Dpm0644 -t $(DESTDIR)$(DATADIR)/dbus-1/system.d stratisd.conf
	install -Dpm0644 -t $(DESTDIR)$(MANDIR)/man8 docs/stratisd.8
	install -Dpm0644 -t $(DESTDIR)$(UDEVDIR)/rules.d udev/11-stratisd.rules
	install -Dpm0644 -t $(DESTDIR)$(UNITDIR) stratisd.service
	install -Dpm0755 -t $(DESTDIR)$(PREFIX)/bin developer_tools/stratis_migrate_symlinks.sh

release:
	RUSTFLAGS="${DENY}" cargo build --release

test-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test loop_

test-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test real_

test-travis:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test travis_

test:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 cargo test -- --skip real_ --skip loop_ --skip travis_

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
	cargo clippy --all-targets --all-features -- ${DENY} ${CLIPPY_DENY}

.PHONY:
	audit
	bloat
	build
	clippy
	docs
	docs-rust
	docs-travis
	fmt
	fmt-travis
	install
	license
	outdated
	release
	test
	test-loop
	test-real
	test-travis
	yamllint
