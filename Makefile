ifeq ($(origin AUDITABLE), undefined)
  BUILD = build
  RUSTC = rustc
else
  BUILD = auditable build
  RUSTC = auditable rustc
endif

ifeq ($(origin CLIPPY_FIX), undefined)
  CLIPPY_OPTS = --all-targets --no-deps
else
  CLIPPY_OPTS = --fix
endif

ifeq ($(origin TARGET), undefined)
else
  TARGET_ARGS = --target=${TARGET}
endif

.DEFAULT_GOAL := help

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

MIN_FEATURES = --no-default-features --features engine,min
NO_IPC_FEATURES = --no-default-features --features engine
SYSTEMD_FEATURES = --no-default-features --features engine,min,systemd_compat
EXTRAS_FEATURES =  --no-default-features --features engine,extras,min
UDEV_FEATURES = --no-default-features --features udev_scripts

DENY = -D warnings -D future-incompatible -D unused -D rust_2018_idioms -D nonstandard_style

STATIC_FLAG = -C target-feature=+crt-static

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

## Run cargo license
license:
	cargo license

## Run cargo audit
audit:
	cargo audit -D warnings

## Audit Rust executables
audit-all-rust: build-all-rust
	cargo audit -D warnings bin \
		./target/${PROFILEDIR}/stratisd \
		./target/${PROFILEDIR}/stratisd-min \
		./target/${PROFILEDIR}/stratis-min \
		./target/${PROFILEDIR}/stratis-utils \
	        ./target/${PROFILEDIR}/stratis-str-cmp \
	        ./target/${PROFILEDIR}/stratis-base32-decode \
	        ./target/${PROFILEDIR}/stratis-dumpmetadata

## Check for spelling errors
check-typos:
	typos

## Run cargo fmt
fmt: fmt-macros
	cargo fmt

## Run cargo fmt for CI jobs
fmt-ci: fmt-macros-ci
	cargo fmt -- --check

## Run cargo fmt for stratisd_proc_macros
fmt-macros:
	cd stratisd_proc_macros && cargo fmt

## Run cargo fmt on stratisd_proc_macros for CI jobs
fmt-macros-ci:
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
	cargo ${BUILD} ${RELEASE_FLAG} \
	--bin=stratisd \
	${TARGET_ARGS}

## Build the stratisd test suite
build-tests:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo test --no-run ${RELEASE_FLAG} ${TARGET_ARGS}

## Build stratisd-min and stratis-min for early userspace
build-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo ${BUILD} ${RELEASE_FLAG} \
	--bin=stratis-min --bin=stratisd-min --bin=stratis-utils \
	${SYSTEMD_FEATURES} ${TARGET_ARGS}

## Build stratisd-min and stratis-min for early userspace
build-no-ipc:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo ${BUILD} ${RELEASE_FLAG} \
	--bin=stratisd \
	${NO_IPC_FEATURES} \
	${TARGET_ARGS}

## Build stratis-str-cmp binary
build-stratis-str-cmp:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo ${RUSTC} ${RELEASE_FLAG}  \
	--bin=stratis-str-cmp \
	${UDEV_FEATURES} \
	${TARGET_ARGS} \
	-- ${STATIC_FLAG}
	@ldd target/${PROFILEDIR}/stratis-str-cmp|grep --quiet --silent "statically linked" || (echo "stratis-str-cmp is not statically linked" && exit 1)

## Build stratis-base32-decode binary
build-stratis-base32-decode:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo ${RUSTC} ${RELEASE_FLAG}  \
	--bin=stratis-base32-decode \
	${UDEV_FEATURES} \
	${TARGET_ARGS} \
	-- ${STATIC_FLAG}
	@ldd target/${PROFILEDIR}/stratis-base32-decode|grep --quiet --silent "statically linked" || (echo "stratis-base32-decode is not statically linked" && exit 1)

## Build stratis-base32-decode and stratis-str-cmp statically
# Extra arguments to `rustc` can only be passed to one target
# so we use two distinct targets to build the two binaries
build-udev-utils: build-stratis-str-cmp build-stratis-base32-decode

## Build the stratis-dumpmetadata program
stratis-dumpmetadata:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo ${BUILD} ${RELEASE_FLAG} \
	--bin=stratis-dumpmetadata ${EXTRAS_FEATURES} ${TARGET_ARGS}

## Build stratis-min for early userspace
stratis-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo ${BUILD} ${RELEASE_FLAG} \
	--bin=stratis-min ${MIN_FEATURES} ${TARGET_ARGS}

## Build stratisd-min for early userspace
stratisd-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo ${BUILD} ${RELEASE_FLAG} \
	--bin=stratisd-min ${SYSTEMD_FEATURES} ${TARGET_ARGS}

## Install udev configuration
install-udev-cfg:
	mkdir -p $(DESTDIR)$(UDEVDIR)/rules.d
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(UDEVDIR)/rules.d udev/61-stratisd.rules

## Install man pages
install-man-cfg:
	mkdir -p $(DESTDIR)$(MANDIR)/man8
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(MANDIR)/man8 docs/stratisd.8
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(MANDIR)/man8 docs/stratis-dumpmetadata.8

## Install dbus config
install-dbus-cfg:
	mkdir -p $(DESTDIR)$(DATADIR)/dbus-1/system.d
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(DATADIR)/dbus-1/system.d stratisd.conf

## Install dracut modules
install-dracut-cfg:
	mkdir -p $(DESTDIR)$(DRACUTDIR)/modules.d
	$(INSTALL) -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' dracut/90stratis/stratisd-min.service.in > $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis/stratisd-min.service
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' dracut/90stratis/module-setup.sh.in > $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis/module-setup.sh
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/stratis-rootfs-setup
	$(INSTALL) -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/61-stratisd.rules
	$(INSTALL) -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/module-setup.sh
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/stratis-clevis-rootfs-setup

## Install systemd configuration
install-systemd-cfg:
	mkdir -p $(DESTDIR)$(UNITDIR)
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' systemd/stratisd.service.in > $(DESTDIR)$(UNITDIR)/stratisd.service
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' systemd/stratisd-min-postinitrd.service.in > $(DESTDIR)$(UNITDIR)/stratisd-min-postinitrd.service
	sed 's|@UNITEXECDIR@|$(UNITEXECDIR)|' systemd/stratis-fstab-setup@.service.in > $(DESTDIR)$(UNITDIR)/stratis-fstab-setup@.service

## Install binaries
install-binaries:
	mkdir -p $(DESTDIR)$(BINDIR)
	mkdir -p $(DESTDIR)$(UNITGENDIR)
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(BINDIR) target/$(PROFILEDIR)/stratis-min
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(BINDIR) target/$(PROFILEDIR)/stratis-utils
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(BINDIR) target/$(PROFILEDIR)/stratis-dumpmetadata
	mv --force --verbose $(DESTDIR)$(BINDIR)/stratis-utils $(DESTDIR)$(BINDIR)/stratis-predict-usage
	ln --force --verbose $(DESTDIR)$(BINDIR)/stratis-predict-usage $(DESTDIR)$(UNITGENDIR)/stratis-clevis-setup-generator
	ln --force --verbose $(DESTDIR)$(BINDIR)/stratis-predict-usage $(DESTDIR)$(UNITGENDIR)/stratis-setup-generator

## Install udev binaries
install-udev-binaries:
	mkdir -p $(DESTDIR)$(UDEVDIR)
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(UDEVDIR) target/$(PROFILEDIR)/stratis-base32-decode
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(UDEVDIR) target/$(PROFILEDIR)/stratis-str-cmp

## Install fstab script
install-fstab-script:
	mkdir -p $(DESTDIR)$(UNITEXECDIR)
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(UNITEXECDIR) systemd/stratis-fstab-setup

## Install daemons
install-daemons:
	mkdir -p $(DESTDIR)$(LIBEXECDIR)
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd
	$(INSTALL) -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd-min

## Install all stratisd files
install: install-udev-cfg install-man-cfg install-dbus-cfg install-dracut-cfg install-systemd-cfg install-binaries install-udev-binaries install-fstab-script install-daemons

## Build all Rust artifacts
build-all-rust: build build-min build-udev-utils stratis-dumpmetadata

## Build all man pages
build-all-man: docs/stratisd.8 docs/stratis-dumpmetadata.8

## Build all stratisd binaries and configuration necessary for install
build-all: build-all-rust build-all-man

## Remove installed configuration files
clean-cfg:
	rm -fv $(DESTDIR)$(DATADIR)/dbus-1/system.d/stratisd.conf
	rm -fv $(DESTDIR)$(MANDIR)/man8/stratisd.8
	rm -fv $(DESTDIR)$(MANDIR)/man8/stratis-dumpmetadata.8
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
	rm -fv $(DESTDIR)$(BINDIR)/stratis-dumpmetadata
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
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test loop_ -- --skip clevis_loop_

## Tests run under valgrind with loop devices
test-loop-valgrind:
	RUST_TEST_THREADS=1 sudo -E valgrind --leak-check=full --num-callers=500 $(shell RUSTFLAGS="${DENY}" cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "'${PWD}/src/lib.rs'") | select(.executable != null) | .executable') loop_ --skip real_ --skip clevis_

## Tests with real devices
test-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test real_ -- --skip clevis_real_

## Basic tests
test:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 cargo test --all-features -- --skip real_ --skip loop_ --skip clevis_ --skip test_stratis_min_ --skip test_stratisd_min_

## Basic tests run under valgrind
test-valgrind:
	RUST_TEST_THREADS=1 valgrind --leak-check=full --num-callers=500 $(shell RUSTFLAGS="${DENY}" cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "'${PWD}/src/lib.rs'") | select(.executable != null) | .executable') --skip real_ --skip loop_ --skip clevis_

## Clevis tests with real devices
test-clevis-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_real_ -- --skip clevis_real_should_fail

## Clevis real device tests that are expected to fail
test-clevis-real-should-fail:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_real_should_fail

## Clevis tests with loop devices
test-clevis-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_loop_ -- --skip clevis_loop_should_fail_

## Clevis tests with loop devices with valgrind
test-clevis-loop-valgrind:
	RUST_TEST_THREADS=1 sudo -E valgrind --leak-check=full --num-callers=500 $(shell RUSTFLAGS="${DENY}" cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "'${PWD}/src/lib.rs'") | select(.executable != null) | .executable') clevis_loop_ --skip clevis_loop_should_fail_

## Clevis loop device tests that are expected to fail
test-clevis-loop-should-fail:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_loop_should_fail_

## Clevis loop device tests that are expected to fail run under valgrind
test-clevis-loop-should-fail-valgrind:
	RUST_TEST_THREADS=1 sudo -E valgrind --leak-check=full --num-callers=500 $(shell RUSTFLAGS="${DENY}" cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "'${PWD}/src/lib.rs'") | select(.executable != null) | .executable') clevis_loop_should_fail_

## Test stratisd-min CLI
test-stratisd-min:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test --no-default-features --features "engine,min" test_stratisd_min

## Test stratis-min CLI
test-stratis-min:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test --no-default-features --features "engine,min" test_stratis_min

## Run yamllint on workflow files
yamllint:
	yamllint --strict .github/workflows/*.yml .packit.yaml

## Run tmt lint
tmtlint:
	tmt lint

## Build docs-rust for CI
docs-ci: docs-rust

## Build rust documentation
docs-rust:
	cargo doc --no-deps

docs/%.8: docs/%.txt
	a2x -f manpage $<

## Run clippy on stratisd_proc_macros
clippy-macros:
	cd stratisd_proc_macros && RUSTFLAGS="${DENY}" cargo clippy --all-features ${CLIPPY_OPTS} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

## Run clippy on the -min build
clippy-min:
	RUSTFLAGS="${DENY}" cargo clippy ${CLIPPY_OPTS} ${MIN_FEATURES} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}
	RUSTFLAGS="${DENY}" cargo clippy ${CLIPPY_OPTS} ${SYSTEMD_FEATURES} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}
	RUSTFLAGS="${DENY}" cargo clippy ${CLIPPY_OPTS} ${EXTRAS_FEATURES} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

## Run clippy on the udev utils
clippy-udev-utils:
	RUSTFLAGS="${DENY}" cargo clippy ${CLIPPY_OPTS} ${UDEV_FEATURES} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

## Run clippy on no-ipc-build
clippy-no-ipc:
	RUSTFLAGS="${DENY}" cargo clippy ${CLIPPY_OPTS} ${NO_IPC_FEATURES} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

## Run clippy on the current source tree
clippy: clippy-macros clippy-min clippy-udev-utils clippy-no-ipc
	RUSTFLAGS="${DENY}" cargo clippy ${CLIPPY_OPTS} -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

.PHONY:
	audit
	audit-all-rust
	build
	build-all
	build-all-man
	build-all-rust
	build-min
	build-udev-utils
	build-stratis-base32-decode
	build-stratis-str-cmp
	check-typos
	clean
	clean-ancillary
	clean-cfg
	clean-primary
	clippy
	clippy-macros
	clippy-min
	clippy-no-ipc
	clippy-udev-utils
	docs-ci
	docs-rust
	fmt
	fmt-ci
	fmt-shell
	fmt-shell-ci
	fmt-macros
	fmt-macros-ci
	help
	install
	install-binaries
	install-daemons
	install-dbus-cfg
	install-dracut-cfg
	install-fstab-script
	install-man-cfg
	install-systemd-cfg
	install-udev-binaries
	install-udev-cfg
	license
	test
	test-valgrind
	test-loop
	test-loop-valgrind
	test-real
	test-clevis-loop
	test-clevis-loop-valgrind
	test-clevis-loop-should-fail
	test-clevis-loop-should-fail-valgrind
	test-clevis-real
	test-clevis-real-should-fail
	tmtlint
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
