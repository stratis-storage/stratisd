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
UNITEXECDIR ?= $(PREFIX)/lib/systemd
UNITGENDIR ?= $(PREFIX)/lib/systemd/system-generators
DRACUTDIR ?= $(PREFIX)/lib/dracut
BINDIR ?= $(PREFIX)/bin

# alternative is "debug"
PROFILEDIR ?= release

ifeq ($(PROFILEDIR), debug)
  PROFILETARGET = build
  PROFILEMINTARGET = build-min
else
  PROFILETARGET = release
  PROFILEMINTARGET = release-min
endif

RELEASE_VERSION ?= 9.9.9

MIN_FEATURES = --no-default-features --features min
SYSTEMD_FEATURES = --no-default-features --features min,systemd_compat
EXTRAS_FEATURES =  --features extras

RUST_2018_IDIOMS = -D bare-trait-objects \
                   -D ellipsis-inclusive-range-patterns

DENY = -D warnings -D future-incompatible -D unused ${RUST_2018_IDIOMS}

# Clippy-related lints
CLIPPY_CARGO = -D clippy::cargo_common_metadata \
               -D clippy::wildcard_dependencies

# Explicitly allow these lints because they don't seem helpful
# doc_markdown: we would rather have useful than well-formatted docs
# from_over_into: preferring from over into is very awkward with JSON report
# map_err_ignore: we generally drop the errors for a reason
# option_if_let_else: causing problems with if-else chains
# similar_names: judges "yes" and "res" to be too similar
# upper_case_acronyms: We use upper case for initialisms, e.g., BDA
CLIPPY_PEDANTIC_USELESS = -A clippy::doc_markdown \
                          -A clippy::from_over_into \
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

install-cfg: docs/stratisd.8
	install -Dpm0644 -t $(DESTDIR)$(DATADIR)/dbus-1/system.d stratisd.conf
	install -Dpm0644 -t $(DESTDIR)$(MANDIR)/man8 docs/stratisd.8
	install -Dpm0644 -t $(DESTDIR)$(UDEVDIR)/rules.d udev/61-stratisd.rules
	install -Dpm0644 -t $(DESTDIR)$(UNITDIR) systemd/stratisd.service
	install -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/dracut.conf.d dracut/90-stratis.conf
	install -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/module-setup.sh
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/stratis-rootfs-setup
	install -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/stratisd-min.service
	install -Dpm0644 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis dracut/90stratis/61-stratisd.rules
	install -Dpm0755 -d $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/module-setup.sh
	install -Dpm0755 -t $(DESTDIR)$(DRACUTDIR)/modules.d/90stratis-clevis dracut/90stratis-clevis/stratis-clevis-rootfs-setup
	install -Dpm0644 -t $(DESTDIR)$(UNITDIR) systemd/stratisd-min-postinitrd.service
	install -Dpm0644 -t $(DESTDIR)$(UNITDIR) systemd/stratis-fstab-setup@.service

install: $(PROFILETARGET) $(PROFILEMINTARGET) install-cfg
	install -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd
	install -Dpm0755 -t $(DESTDIR)$(UDEVDIR) target/$(PROFILEDIR)/stratis-utils
	mv -fv $(DESTDIR)$(UDEVDIR)/stratis-utils $(DESTDIR)$(UDEVDIR)/stratis-str-cmp
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UDEVDIR)/stratis-base32-decode
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(BINDIR)/stratis-predict-usage
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UNITGENDIR)/stratis-clevis-setup-generator
	ln -fv $(DESTDIR)$(UDEVDIR)/stratis-str-cmp $(DESTDIR)$(UNITGENDIR)/stratis-setup-generator
	install -Dpm0755 -t $(DESTDIR)$(PREFIX)/bin target/$(PROFILEDIR)/stratis-min
	install -Dpm0755 -t $(DESTDIR)$(LIBEXECDIR) target/$(PROFILEDIR)/stratisd-min
	install -Dpm0755 -t $(DESTDIR)$(UNITEXECDIR) systemd/stratis-fstab-setup

# remove installed configuration files
clean-cfg:
	rm -fv $(DESTDIR)$(DATADIR)/dbus-1/system.d/stratisd.conf
	rm -fv $(DESTDIR)$(MANDIR)/man8/stratisd.8
	rm -fv $(DESTDIR)$(UDEVDIR)/rules.d/*-stratisd.rules
	rm -fv $(DESTDIR)$(UNITDIR)/stratisd.service
	rm -fv $(DESTDIR)$(DRACUTDIR)/dracut.conf.d/90-stratis.conf
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
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_real_

test-clevis-loop:
	RUSTFLAGS="${DENY}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_loop_

yamllint:
	yamllint --strict .github/workflows/*.yml

docs-travis: docs-rust

docs-rust:
	cargo doc --no-deps

docs/stratisd.8: docs/stratisd.txt
	a2x -f manpage docs/stratisd.txt

clippy:
	RUSTFLAGS="${DENY}" cargo clippy --all-targets -- ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS} ${CLIPPY_CARGO}
	RUSTFLAGS="${DENY}" cargo clippy --all-targets ${MIN_FEATURES} -- ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS} ${CLIPPY_CARGO}
	RUSTFLAGS="${DENY}" cargo clippy --all-targets ${SYSTEMD_FEATURES} -- ${CLIPPY_PEDANTIC} ${CLIPPY_PEDANTIC_USELESS} ${CLIPPY_CARGO}

set-lower-bounds:
	${PWD}/code_maintenance/set_lower_bounds

# Note that this target is really just a helper target for the
# verify-dependency-bounds target.
build-all:
	PKG_CONFIG_ALLOW_CROSS=1 \
	RUSTFLAGS="${DENY}" \
	cargo build --all-targets --all-features

# Verify that the dependency bounds set in Cargo.toml are not lower
# than is actually reqired. Use build-all target to set up for cargo-tree
# and also to test that everything still compiles when the versions are set
# to their precise values.
verify-dependency-bounds:
	$(MAKE) build-all
	$(MAKE) set-lower-bounds
	$(MAKE) build-all

# Check that there are no dependencies missing in Fedora or ones that require
# versions higher than those available in Fedora. There is no reason to fail
# if our versions are too low, as we can bump versions at our leisure, so
# do not check the value of the low key. The jq filter can be updated if we
# deliberately choose a too high or a missing dependency.
check-fedora-versions:
	`${COMPARE_FEDORA_VERSIONS} | jq '[.missing == [], .high == []] | all'`

.PHONY:
	audit
	bloat
	build
	build-all
	build-min
	check-fedora-versions
	clean
	clean-ancillary
	clean-cfg
	clean-primary
	clippy
	create-release
	docs-rust
	docs-travis
	fmt
	fmt-travis
	install
	install-cfg
	license
	outdated
	release
	release-min
	set-lower-bounds
	test
	test-loop
	test-real
	test-clevis-loop
	test-clevis-real
	verify-dependency-bounds
	yamllint
