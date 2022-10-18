ifeq ($(origin TARGET), undefined)
else
  TARGET_ARGS = --target=${TARGET}
endif

.DEFAULT_GOAL := help

IGNORE_ARGS ?=

INSTALL ?= /usr/bin/install

DESTDIR ?=
PREFIX ?= /usr
LIBEXECDIR ?= $(PREFIX)/libexec
DATADIR ?= $(PREFIX)/share
UDEVDIR ?= $(PREFIX)/lib/udev
MANDIR ?= $(DATADIR)/man
UNITDIR ?= $(PREFIX)/lib/systemd/system
UNITEXECDIR ?= $(PREFIX)/lib/systemd
UNITGENDIR ?= $(PREFIX)/lib/systemd/system-generators
DRACUTDIR ?= $(PREFIX)/lib/dracut
BINDIR ?= $(PREFIX)/bin

# alternative is "debug"
PROFILEDIR ?= release

ifeq ($(PROFILEDIR), debug)
  RELEASE_FLAG =
else
  RELEASE_FLAG = --release
endif

MIN_FEATURES = --no-default-features --features min
SYSTEMD_FEATURES = --no-default-features --features min,systemd_compat
EXTRAS_FEATURES =  --no-default-features --features extras,min

DENY = -D warnings -D future-incompatible -D unused -D rust_2018_idioms -D nonstandard_style

CLIPPY_DENY = -D clippy::all -D clippy::cargo -A clippy::multiple-crate-versions

# Explicitly allow these lints because they don't seem helpful
# doc_markdown: we would rather have useful than well-formatted docs
# from_over_into: preferring from over into is very awkward with JSON report
# manual_filter_map: sometimes map() after filter_map() is clearer
# map_err_ignore: we generally drop the errors for a reason
# option_if_let_else: causing problems with if-else chains
# similar_names: judges "yes" and "res" to be too similar
# upper_case_acronyms: We use upper case for initialisms, e.g., BDA
CLIPPY_PEDANTIC_USELESS = -A clippy::doc_markdown \
                          -A clippy::from_over_into \
                          -A clippy::manual_filter_map \
                          -A clippy::map_err_ignore \
                          -A clippy::option_if_let_else \
                          -A clippy::similar_names \
                          -A clippy::upper_case_acronyms \

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
                  -D clippy::unused_self \
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

${HOME}/.cargo/bin/cargo-expand:
	cargo install cargo-expand

## Run cargo outdated
outdated: ${HOME}/.cargo/bin/cargo-outdated
	PATH=${HOME}/.cargo/bin:${PATH} cargo outdated

## Run cargo license
license: ${HOME}/.cargo/bin/cargo-license
	PATH=${HOME}/.cargo/bin:${PATH} cargo license

## Run cargo bloat
bloat: ${HOME}/.cargo/bin/cargo-bloat
	PATH=${HOME}/.cargo/bin:${PATH} cargo bloat --release
	PATH=${HOME}/.cargo/bin:${PATH} cargo bloat --release --crates

## Run cargo audit
audit: ${HOME}/.cargo/bin/cargo-audit
	PATH=${HOME}/.cargo/bin:${PATH} cargo audit -D warnings

## Run cargo expand
expand: ${HOME}/.cargo/bin/cargo-expand
	PATH=${HOME}/.cargo/bin:${PATH} cargo expand --lib engine::strat_engine::pool

## Run cargo fmt
fmt: fmt-macros
	cargo fmt

## Run cargo fmt for stratisd_proc_macros
fmt-macros:
	cd stratisd_proc_macros && cargo fmt

## Run cargo fmt for CI jobs
fmt-travis: fmt-macros-travis
	cargo fmt -- --check

## Run cargo fmt on stratisd_proc_macros for CI jobs
fmt-macros-travis:
	cd stratisd_proc_macros && cargo fmt -- --check

## Check shell formatting with shfmt
fmt-shell:
	shfmt -l -w .

## Check shell formatting with shfmt for CI
fmt-shell-ci:
	shfmt -d .

## Build stratisd
build:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${RELEASE_FLAG} ${TARGET_ARGS}

## Build the stratisd test suite
build-tests:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo test --no-run ${RELEASE_FLAG} ${TARGET_ARGS}

## Build stratisd extra binaries
build-extras:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${RELEASE_FLAG} ${EXTRAS_FEATURES} ${TARGET_ARGS}

## Build stratisd-min and stratis-min for early userspace
build-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${RELEASE_FLAG} \
	--bin=stratis-min --bin=stratisd-min --bin=stratis-utils \
	${SYSTEMD_FEATURES} ${TARGET_ARGS}

## Build the stratis-dumpmetadata program
stratis-dumpmetadata:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${RELEASE_FLAG} \
	--bin=stratis_dumpmetadata ${EXTRAS_FEATURES} ${TARGET_ARGS}

## Build stratis-min for early userspace
stratis-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${RELEASE_FLAG} \
	--bin=stratis-min ${MIN_FEATURES} ${TARGET_ARGS}

## Build stratisd-min for early userspace
stratisd-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${RELEASE_FLAG} \
	--bin=stratisd-min ${SYSTEMD_FEATURES} ${TARGET_ARGS}

## Install stratisd configuration
install-cfg:
	mkdir -p $(DESTDIR)$(UNITDIR)
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(DATADIR)/dbus-1/system.d stratisd.conf
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(MANDIR)/man8 docs/stratisd.8
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(UDEVDIR)/rules.d udev/61-stratisd.rules
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' systemd/stratisd.service.in > $(DESTDIR)$(UNITDIR)/stratisd.service
	$(INSTALL) -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/module-setup.sh
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/stratis-rootfs-setup
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/stratisd-min.service
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/61-stratisd.rules
	$(INSTALL) -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/module-setup.sh
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/stratis-clevis-rootfs-setup
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' systemd/stratisd-min-postinitrd.service.in > $(DESTDIR)$(UNITDIR)/stratisd-min-postinitrd.service
	sed 's|@UNITEXECDIR@|$(UNITEXECDIR)|' systemd/stratis-fstab-setup@.service.in > $(DESTDIR)$(UNITDIR)/stratis-fstab-setup@.service

## Install stratisd binaries and configuration
install: install-cfg
	mkdir -p $(DESTDIR)$(UNITGENDIR)
	mkdir -p $(DESTDIR)$(BINDIR)
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(UDEVDIR) target/$(PROFILEDIR)/stratis-utils
	mv -fv $(DESTDIR)$(UDEVDIR)/stratis-utils $(DESTDIR)$(UDEVDIR)/stratis-str-cmp
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UDEVDIR)/stratis-base32-decode
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(BINDIR)/stratis-predict-usage
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UNITGENDIR)/stratis-clevis-setup-generator
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UNITGENDIR)/stratis-setup-generator
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(BINDIR) target/$(PROFILEDIR)/stratis-min
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd-min
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(UNITEXECDIR) systemd/stratis-fstab-setup

## Build and install stratisd binaries and configuration
build-and-install: build build-min docs/stratisd.8 install

## Remove installed configuration files
clean-cfg:
	rm -fv $(DESTDIR)$(DATADIR)/dbus-1/system.d/stratisd.conf
	rm -fv $(DESTDIR)$(MANDIR)/man8/stratisd.8
	rm -fv $(DESTDIR)$(UDEVDIR)/rules.d/*-stratisd.rules
	rm -fv $(DESTDIR)$(UNITDIR)/stratisd.service
	rm -rfv $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis
	rm -rfv $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis
	rm -fv $(DESTDIR)$(UNITDIR)/stratisd-min-postinitrd.service
	rm -fv $(DESTDIR)$(UNITDIR)/stratis-fstab-setup@.service

## Remove installed non-primary tools generated by the build process
clean-ancillary:
	rm -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp
	rm -fv $(DESTDIR)$(UDEVDIR)/stratis-base32-decode
	rm -fv $(DESTDIR)$(BINDIR)/stratis-predict-usage
	rm -fv $(DESTDIR)$(UNITGENDIR)/stratis-setup-generator
	rm -fv $(DESTDIR)$(UNITGENDIR)/stratis-clevis-setup-generator
	rm -fv $(DESTDIR)$(UNITEXECDIR)/stratis-fstab-setup

## Remove installed command-line tools and daemons generated by the build process
clean-primary:
	rm -fv $(DESTDIR)$(LIBEXECDIR)/stratisd
	rm -fv $(DESTDIR)$(PREFIX)/stratis-min
	rm -fv $(DESTDIR)$(LIBEXECDIR)/stratisd-min

## Remove installed items
clean: clean-cfg clean-ancillary clean-primary

## Tests with loop devices
test-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test loop_ -- --skip clevis_loop_

## Tests with real devices
test-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test real_ -- --skip clevis_real_

## Basic tests
test:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 cargo test --all-features -- --skip real_ --skip loop_ --skip clevis_

## Clevis tests with real devices
test-clevis-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_real_ -- --skip clevis_real_should_fail

## Clevis real device tests that are expected to fail
test-clevis-real-should-fail:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_real_should_fail

## Clevis tests with loop devices
test-clevis-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_loop_ -- --skip clevis_loop_should_fail_

## Clevis loop device tests that are expected to fail
test-clevis-loop-should-fail:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_loop_should_fail_

## Run yamllint on workflow files
yamllint:
	yamllint --strict .github/workflows/*.yml

## Build docs-rust for CI
docs-travis: docs-rust

## Build rust documentation
docs-rust:
	cargo doc --no-deps

docs/%.8: docs/%.txt
	a2x -f manpage $<

## Run clippy on stratisd_proc_macros
clippy-macros:
	cd stratisd_proc_macros && RUSTFLAGS="${DENY}" cargo clippy --all-targets --all-features -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

## Run clippy on the current source tree
clippy: clippy-macros
	RUSTFLAGS="${DENY}" cargo clippy --all-targets -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}
	RUSTFLAGS="${DENY}" cargo clippy --all-targets ${MIN_FEATURES} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}
	RUSTFLAGS="${DENY}" cargo clippy --all-targets ${SYSTEMD_FEATURES} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

.PHONY:
	audit
	bloat
	build
	build-min
	clean
	clean-ancillary
	clean-cfg
	clean-primary
	clippy
	clippy-macros
	docs-rust
	docs-travis
	expand
	fmt
	fmt-shell
	fmt-shell-ci
	fmt-travis
	fmt-macros
	fmt-macros-travis
	help
	install
	install-cfg
	license
	outdated
	test
	test-loop
	test-real
	test-clevis-loop
	test-clevis-loop-should-fail
	test-clevis-real
	test-clevis-real-should-fail
	yamllint

# COLORS
GREEN  := $(shell tput -Txterm setaf 2)
YELLOW := $(shell tput -Txterm setaf 3)
WHITE  := $(shell tput -Txterm setaf 7)
RESET  := $(shell tput -Txterm sgr0)


TARGET_MAX_CHARS=30
## Show help
help:
	@echo ''
	@echo 'Usage:'
	@echo '  ${YELLOW}make${RESET} ${GREEN}<target>${RESET}'
	@echo ''
	@echo 'Targets:'
	@awk '/^[a-zA-Z\-_0-9]+:/ { \
		targetHelp = match(lastLine, /^## (.*)/); \
		if (targetHelp) { \
			target = substr($$1, 0, index($$1, ":")-1); \
			targetHelp = substr(lastLine, RSTART + 3, RLENGTH); \
			printf "  ${YELLOW}%-$(TARGET_MAX_CHARS)s${RESET} ${GREEN}%s${RESET}\n", target, targetHelp; \
		} \
	} \
	{ lastLine = $$0 }' $(MAKEFILE_LIST)
