ifeq ($(origin TARGET), undefined)
else
  TARGET_ARGS = --target=${TARGET}
endif

ifeq ($(origin MANIFEST_PATH), undefined)
else
  MANIFEST_PATH_ARGS = --manifest-path=${MANIFEST_PATH}
endif

ifeq ($(origin FEDORA_RELEASE), undefined)
else
  FEDORA_RELEASE_ARGS = --release=${FEDORA_RELEASE}
endif

IGNORE_ARGS ?=

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

MIN_FEATURES = --no-default-features --features min
SYSTEMD_FEATURES = --no-default-features --features min,systemd_compat
EXTRAS_FEATURES =  --features extras

DENY = -D warnings -D future-incompatible -D unused -D rust_2018_idioms -D nonstandard_style

CLIPPY_DENY = -D clippy::all -D clippy::cargo

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

outdated: ${HOME}/.cargo/bin/cargo-outdated
	PATH=${HOME}/.cargo/bin:${PATH} cargo outdated

license: ${HOME}/.cargo/bin/cargo-license
	PATH=${HOME}/.cargo/bin:${PATH} cargo license

bloat: ${HOME}/.cargo/bin/cargo-bloat
	PATH=${HOME}/.cargo/bin:${PATH} cargo bloat --release
	PATH=${HOME}/.cargo/bin:${PATH} cargo bloat --release --crates

audit: ${HOME}/.cargo/bin/cargo-audit
	# Remove --ignore when bindgen dependency is increased to ^0.60
	PATH=${HOME}/.cargo/bin:${PATH} cargo audit -D warnings --ignore=RUSTSEC-2021-0139

expand: ${HOME}/.cargo/bin/cargo-expand
	PATH=${HOME}/.cargo/bin:${PATH} cargo expand --lib engine::strat_engine::pool

fmt: fmt-macros
	cargo fmt

fmt-macros:
	cd stratisd_proc_macros && cargo fmt

fmt-travis: fmt-macros-travis
	cargo fmt -- --check

fmt-macros-travis:
	cd stratisd_proc_macros && cargo fmt -- --check

fmt-shell:
	shfmt -l -w .

fmt-shell-ci:
	shfmt -d .

build:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${TARGET_ARGS}

build-tests:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo test --no-run ${TARGET_ARGS}

build-extras:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${EXTRAS_FEATURES} ${TARGET_ARGS}

build-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis-min --bin=stratisd-min --bin=stratis-utils \
	${SYSTEMD_FEATURES} ${TARGET_ARGS}

release-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --release --bin=stratis-min --bin=stratisd-min \
	--bin=stratis-utils ${SYSTEMD_FEATURES} ${TARGET_ARGS}

stratis-dumpmetadata:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis_dumpmetadata ${EXTRAS_FEATURES} ${TARGET_ARGS}

stratis-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratis-min ${MIN_FEATURES} ${TARGET_ARGS}

stratisd-min:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --bin=stratisd-min ${SYSTEMD_FEATURES} ${TARGET_ARGS}

install-cfg:
	mkdir -p $(DESTDIR)$(UNITDIR)
	install -Dpm0644 -t $(DESTDIR)$(DATADIR)/dbus-1/system.d stratisd.conf
	install -Dpm0644 -t $(DESTDIR)$(MANDIR)/man8 docs/stratisd.8
	install -Dpm0644 -t $(DESTDIR)$(UDEVDIR)/rules.d udev/61-stratisd.rules
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' systemd/stratisd.service.in > $(DESTDIR)$(UNITDIR)/stratisd.service
	install -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/module-setup.sh
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/stratis-rootfs-setup
	install -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/stratisd-min.service
	install -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/61-stratisd.rules
	install -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/module-setup.sh
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/stratis-clevis-rootfs-setup
	sed 's|@LIBEXECDIR@|$(LIBEXECDIR)|' systemd/stratisd-min-postinitrd.service.in > $(DESTDIR)$(UNITDIR)/stratisd-min-postinitrd.service
	sed 's|@UNITEXECDIR@|$(UNITEXECDIR)|' systemd/stratis-fstab-setup@.service.in > $(DESTDIR)$(UNITDIR)/stratis-fstab-setup@.service

install: install-cfg
	mkdir -p $(DESTDIR)$(UNITGENDIR)
	mkdir -p $(DESTDIR)$(BINDIR)
	install -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd
	install -Dpm0755 -t $(DESTDIR)$(UDEVDIR) target/$(PROFILEDIR)/stratis-utils
	mv -fv $(DESTDIR)$(UDEVDIR)/stratis-utils $(DESTDIR)$(UDEVDIR)/stratis-str-cmp
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UDEVDIR)/stratis-base32-decode
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(BINDIR)/stratis-predict-usage
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UNITGENDIR)/stratis-clevis-setup-generator
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UNITGENDIR)/stratis-setup-generator
	install -Dpm0755 -t $(DESTDIR)$(BINDIR) target/$(PROFILEDIR)/stratis-min
	install -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd-min
	install -Dpm0755 -t $(DESTDIR)$(UNITEXECDIR) systemd/stratis-fstab-setup

install-release: release release-min docs/stratisd.8
	${MAKE} install

install-debug: build build-min docs/stratisd.8
	${MAKE} install PROFILEDIR=debug

# remove installed configuration files
clean-cfg:
	rm -fv $(DESTDIR)$(DATADIR)/dbus-1/system.d/stratisd.conf
	rm -fv $(DESTDIR)$(MANDIR)/man8/stratisd.8
	rm -fv $(DESTDIR)$(UDEVDIR)/rules.d/*-stratisd.rules
	rm -fv $(DESTDIR)$(UNITDIR)/stratisd.service
	rm -rfv $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis
	rm -rfv $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis
	rm -fv $(DESTDIR)$(UNITDIR)/stratisd-min-postinitrd.service
	rm -fv $(DESTDIR)$(UNITDIR)/stratis-fstab-setup@.service

# remove installed non-primary tools generated by the build process
clean-ancillary:
	rm -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp
	rm -fv $(DESTDIR)$(UDEVDIR)/stratis-base32-decode
	rm -fv $(DESTDIR)$(BINDIR)/stratis-predict-usage
	rm -fv $(DESTDIR)$(UNITGENDIR)/stratis-setup-generator
	rm -fv $(DESTDIR)$(UNITGENDIR)/stratis-clevis-setup-generator
	rm -fv $(DESTDIR)$(UNITEXECDIR)/stratis-fstab-setup

# remove installed command-line tools and daemons generated by the build process
clean-primary:
	rm -fv $(DESTDIR)$(LIBEXECDIR)/stratisd
	rm -fv $(DESTDIR)$(PREFIX)/stratis-min
	rm -fv $(DESTDIR)$(LIBEXECDIR)/stratisd-min

# remove installed items
clean: clean-cfg clean-ancillary clean-primary

release:
	RUSTFLAGS="${DENY}" cargo build --release

test-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test loop_ -- --skip clevis_loop_

test-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test real_ -- --skip clevis_real_

test:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 cargo test --all-features -- --skip real_ --skip loop_ --skip clevis_

test-clevis-real:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_real_ -- --skip clevis_real_should_fail

test-clevis-real-should-fail:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_real_should_fail

test-clevis-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_loop_ -- --skip clevis_loop_should_fail_

test-clevis-loop-should-fail:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_loop_should_fail_

yamllint:
	yamllint --strict .github/workflows/*.yml

docs-travis: docs-rust

docs-rust:
	cargo doc --no-deps

docs/stratisd.8: docs/stratisd.txt
	a2x -f manpage docs/stratisd.txt

clippy-macros:
	cd stratisd_proc_macros && RUSTFLAGS="${DENY}" cargo clippy --all-targets --all-features -- ${CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

# stratisd requires the most recent version of libmount, which is over 2 years
# old and requires some older versions of nix and cfg-if which are not
# semantic version compatible with the new ones that stratisd requires
STRATISD_CLIPPY_DENY = ${CLIPPY_DENY} -A clippy::multiple-crate-versions
clippy: clippy-macros
	RUSTFLAGS="${DENY}" cargo clippy --all-targets -- ${STRATISD_CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}
	RUSTFLAGS="${DENY}" cargo clippy --all-targets ${MIN_FEATURES} -- ${STRATISD_CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}
	RUSTFLAGS="${DENY}" cargo clippy --all-targets ${SYSTEMD_FEATURES} -- ${STRATISD_CLIPPY_DENY} ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS}

SET_LOWER_BOUNDS ?=
test-set-lower-bounds:
	echo "Testing that SET_LOWER_BOUNDS environment variable is set to a valid path"
	test -e "${SET_LOWER_BOUNDS}"

verify-dependency-bounds: test-set-lower-bounds
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${MANIFEST_PATH_ARGS} --all-targets --all-features
	${SET_LOWER_BOUNDS} ${MANIFEST_PATH_ARGS}
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build ${MANIFEST_PATH_ARGS} --all-targets --all-features
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo test --no-run ${TARGET_ARGS}
	${SET_LOWER_BOUNDS} ${MANIFEST_PATH_ARGS}
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo test --no-run ${TARGET_ARGS}

COMPARE_FEDORA_VERSIONS ?=
test-compare-fedora-versions:
	echo "Testing that COMPARE_FEDORA_VERSIONS environment variable is set to a valid path"
	test -e "${COMPARE_FEDORA_VERSIONS}"

check-fedora-versions: test-compare-fedora-versions
	${COMPARE_FEDORA_VERSIONS} ${MANIFEST_PATH_ARGS} ${FEDORA_RELEASE_ARGS} ${IGNORE_ARGS}

.PHONY:
	audit
	bloat
	build
	build-min
	check-fedora-versions
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
	install
	install-cfg
	license
	outdated
	release
	release-min
	test
	test-loop
	test-real
	test-clevis-loop
	test-clevis-loop-should-fail
	test-clevis-real
	test-clevis-real-should-fail
	test-compare-fedora-versions
	test-set-lower-bounds
	verify-dependency-bounds
	yamllint
