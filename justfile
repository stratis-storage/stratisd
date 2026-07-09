# Installation configuration
DESTDIR := env_var_or_default('DESTDIR', '')
PREFIX := env_var_or_default('PREFIX', '/usr')
LIBEXECDIR := DESTDIR + PREFIX / 'libexec'
DATADIR := DESTDIR + PREFIX / 'share'
UDEVDIR := DESTDIR + PREFIX / 'lib/udev'
MANDIR := DESTDIR + DATADIR / 'man'
UNITDIR := DESTDIR + PREFIX / 'lib/systemd/system'
UNITEXECDIR := DESTDIR + PREFIX / 'lib/systemd'
UNITGENDIR := DESTDIR + PREFIX / 'lib/systemd/system-generators'
DRACUTDIR := DESTDIR + PREFIX / 'lib/dracut'
BINDIR := DESTDIR + PREFIX / 'bin'
INSTALL := env_var_or_default('INSTALL', '/usr/bin/install')

# Build configuration
PROFILEDIR := env_var_or_default('PROFILEDIR', 'release')
RELEASE_FLAG := if PROFILEDIR == 'debug' { '' } else { '--release' }

# Feature sets
MIN_FEATURES := '--no-default-features --features engine,min'
NO_IPC_FEATURES := '--no-default-features --features engine'
SYSTEMD_FEATURES := '--no-default-features --features engine,min,systemd_compat'
EXTRAS_FEATURES := '--no-default-features --features engine,extras'
UDEV_FEATURES := '--no-default-features --features udev_scripts'
UTILS_FEATURES := '--no-default-features --features dbus_enabled,engine,systemd_compat'

# Compiler flags
STATIC_FLAG := '-C target-feature=+crt-static'
PROFILE_FLAGS := env_var_or_default('PROFILE_FLAGS', '')
CAP_LINTS_FLAGS := if env_var_or_default('CAP_LINTS', '') != '' { '--cap-lints=' + env_var('CAP_LINTS') } else { '' }
RUSTFLAGS := PROFILE_FLAGS + ' ' + CAP_LINTS_FLAGS

# Cargo commands (conditional on build mode)
BUILD := if env_var_or_default('AUDITABLE', '') != '' { 'auditable build' } else if env_var_or_default('MINIMAL', '') != '' { 'minimal-versions build --direct' } else { 'build' }
CLIPPY := if env_var_or_default('AUDITABLE', '') != '' { 'clippy' } else if env_var_or_default('MINIMAL', '') != '' { 'minimal-versions clippy --direct' } else { 'clippy' }
RUSTC := if env_var_or_default('AUDITABLE', '') != '' { 'auditable rustc' } else { 'rustc' }
TEST := if env_var_or_default('AUDITABLE', '') != '' { 'test' } else if env_var_or_default('MINIMAL', '') != '' { 'minimal-versions test --direct' } else { 'test' }

# Other configuration
CLIPPY_OPTS := if env_var_or_default('CLIPPY_FIX', '') != '' { '--fix' } else { '--all-targets --no-deps' }
AUDIT_OPTS := '-D warnings'
TARGET_ARGS := if env_var_or_default('TARGET', '') != '' { '--target=' + env_var('TARGET') } else { '' }
FEDORA_RELEASE_ARGS := if env_var_or_default('FEDORA_RELEASE', '') != '' { '--release=' + env_var('FEDORA_RELEASE') } else { '' }
IGNORE_ARGS := env_var_or_default('IGNORE_ARGS', '')
COMPARE_FEDORA_VERSIONS := env_var_or_default('COMPARE_FEDORA_VERSIONS', '')

# Default recipe
_default:
    @just --list

# Run cargo audit
audit:
    cargo audit {{AUDIT_OPTS}}

# Audit Rust executables
audit-all-rust: build-all-rust
    cargo audit {{AUDIT_OPTS}} bin \
        ./target/{{PROFILEDIR}}/stratisd \
        ./target/{{PROFILEDIR}}/stratisd-min \
        ./target/{{PROFILEDIR}}/stratis-min \
        ./target/{{PROFILEDIR}}/stratis-utils \
        ./target/{{PROFILEDIR}}/stratis-str-cmp \
        ./target/{{PROFILEDIR}}/stratis-base32-decode \
        ./target/{{PROFILEDIR}}/stratisd-tools

# Build all stratisd binaries and configuration necessary for install
build-all: build-all-rust build-all-man

# Build all man pages
build-all-man: (build-man 'stratisd') (build-man 'stratis-dumpmetadata')

# Build a man page from asciidoc
build-man page:
    a2x -f manpage docs/{{page}}.txt

# Build all Rust artifacts
build-all-rust: stratisd build-min build-utils build-udev-utils stratisd-tools

# Build stratisd-min and stratis-min for early userspace
build-min:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratis-min --bin=stratisd-min \
    {{SYSTEMD_FEATURES}} {{TARGET_ARGS}}

# Build min targets without systemd support enabled
build-min-no-systemd:
    PKG_CONFIG_ALLOW_CROSS=1 \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratis-min --bin=stratisd-min \
    {{MIN_FEATURES}} {{TARGET_ARGS}}

# Build stratisd without IPC support
build-no-ipc:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratisd \
    {{NO_IPC_FEATURES}} \
    {{TARGET_ARGS}}

# Build the stratisd test suite
build-tests:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{TEST}} --no-run {{RELEASE_FLAG}} {{TARGET_ARGS}}

# Build stratis-base32-decode and stratis-str-cmp statically
build-udev-utils: stratis-str-cmp stratis-base32-decode

# Build stratis-utils only
build-utils:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratis-utils \
    {{UTILS_FEATURES}} {{TARGET_ARGS}}

# Build stratis-utils without systemd
build-utils-no-systemd:
    PKG_CONFIG_ALLOW_CROSS=1 \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratis-utils \
    {{NO_IPC_FEATURES}} {{TARGET_ARGS}}

# Verify that the dependency specs are satisfied in Fedora
check-fedora-versions: test-compare-fedora-versions
    {{COMPARE_FEDORA_VERSIONS}} {{FEDORA_RELEASE_ARGS}} {{IGNORE_ARGS}}

# Check for spelling errors
check-typos:
    typos

# Remove installed items
clean: clean-cfg clean-ancillary clean-primary

# Remove installed non-primary tools generated by the build process
clean-ancillary:
    rm -fv {{UDEVDIR}}/stratis-str-cmp
    rm -fv {{UDEVDIR}}/stratis-base32-decode
    rm -fv {{BINDIR}}/stratis-predict-usage
    rm -fv {{BINDIR}}/stratisd-tools
    rm -fv {{BINDIR}}/stratis-dumpmetadata
    rm -fv {{BINDIR}}/stratis-decode-dm
    rm -fv {{UNITGENDIR}}/stratis-setup-generator
    rm -fv {{UNITGENDIR}}/stratis-clevis-setup-generator
    rm -fv {{UNITEXECDIR}}/stratis-fstab-setup

# Remove installed configuration files
clean-cfg:
    rm -fv {{DATADIR}}/dbus-1/system.d/stratisd.conf
    rm -fv {{MANDIR}}/man8/stratisd.8
    rm -fv {{MANDIR}}/man8/stratis-dumpmetadata.8
    rm -fv {{UDEVDIR}}/rules.d/*-stratisd.rules
    rm -fv {{UNITDIR}}/stratisd.service
    rm -rfv {{DRACUTDIR}}/modules.d/50stratis
    rm -rfv {{DRACUTDIR}}/modules.d/50stratis-clevis
    rm -fv {{UNITDIR}}/stratisd-min-postinitrd.service
    rm -fv {{UNITDIR}}/stratis-fstab-setup@.service
    rm -fv {{UNITDIR}}/stratis-fstab-setup-with-network@.service

# Remove installed command-line tools and daemons generated by the build process
clean-primary:
    rm -fv {{LIBEXECDIR}}/stratisd
    rm -fv {{PREFIX}}/stratis-min
    rm -fv {{LIBEXECDIR}}/stratisd-min

# Run clippy on the current source tree
clippy: clippy-macros clippy-min clippy-udev-utils clippy-no-ipc clippy-utils
    cargo {{CLIPPY}} {{CLIPPY_OPTS}}

# Run clippy on stratisd_proc_macros
clippy-macros:
    cd stratisd_proc_macros && cargo {{CLIPPY}} --all-features {{CLIPPY_OPTS}}

# Run clippy on the -min build
clippy-min:
    cargo {{CLIPPY}} {{CLIPPY_OPTS}} {{MIN_FEATURES}}
    cargo {{CLIPPY}} {{CLIPPY_OPTS}} {{SYSTEMD_FEATURES}}
    cargo {{CLIPPY}} {{CLIPPY_OPTS}} {{EXTRAS_FEATURES}}

# Run clippy on no-ipc-build
clippy-no-ipc:
    cargo {{CLIPPY}} {{CLIPPY_OPTS}} {{NO_IPC_FEATURES}}

# Run clippy on the udev utils
clippy-udev-utils:
    cargo {{CLIPPY}} {{CLIPPY_OPTS}} {{UDEV_FEATURES}}

# Run clippy on the utils binary
clippy-utils:
    cargo {{CLIPPY}} {{CLIPPY_OPTS}} {{UTILS_FEATURES}}

# Build docs-rust for CI
docs-ci: docs-rust

# Build rust documentation
docs-rust:
    cargo doc --no-deps

# Run cargo fmt
fmt: fmt-macros
    cargo fmt

# Run cargo fmt for CI jobs
fmt-ci: fmt-macros-ci
    cargo fmt -- --check

# Run cargo fmt for stratisd_proc_macros
fmt-macros:
    cd stratisd_proc_macros && cargo fmt

# Run cargo fmt on stratisd_proc_macros for CI jobs
fmt-macros-ci:
    cd stratisd_proc_macros && cargo fmt -- --check

# Check shell formatting with shfmt
fmt-shell:
    shfmt -l -w .

# Check shell formatting with shfmt for CI
fmt-shell-ci:
    shfmt -d .

# Install all stratisd files
install: install-programs install-man-cfg

# Install binaries
install-binaries:
    mkdir -p {{BINDIR}}
    mkdir -p {{UNITGENDIR}}
    {{INSTALL}} -Dpm0755 -t {{BINDIR}} target/{{PROFILEDIR}}/stratis-min
    {{INSTALL}} -Dpm0755 -t {{BINDIR}} target/{{PROFILEDIR}}/stratisd-tools
    ln --force --verbose {{BINDIR}}/stratisd-tools {{BINDIR}}/stratis-dumpmetadata
    {{INSTALL}} -Dpm0755 -t {{BINDIR}} target/{{PROFILEDIR}}/stratis-utils
    mv --force --verbose {{BINDIR}}/stratis-utils {{BINDIR}}/stratis-predict-usage
    ln --force --verbose {{BINDIR}}/stratis-predict-usage {{UNITGENDIR}}/stratis-clevis-setup-generator
    ln --force --verbose {{BINDIR}}/stratis-predict-usage {{UNITGENDIR}}/stratis-setup-generator
    ln --force --verbose {{BINDIR}}/stratis-predict-usage {{BINDIR}}/stratis-decode-dm

# Install daemons
install-daemons:
    mkdir -p {{LIBEXECDIR}}
    {{INSTALL}} -Dpm0755 -t {{LIBEXECDIR}} target/{{PROFILEDIR}}/stratisd
    {{INSTALL}} -Dpm0755 -t {{LIBEXECDIR}} target/{{PROFILEDIR}}/stratisd-min

# Install dbus config
install-dbus-cfg:
    mkdir -p {{DATADIR}}/dbus-1/system.d
    {{INSTALL}} -Dpm0644 -t {{DATADIR}}/dbus-1/system.d stratisd.conf

# Install dracut modules
install-dracut-cfg:
    mkdir -p {{DRACUTDIR}}/modules.d
    {{INSTALL}} -Dpm0755 -d {{DRACUTDIR}}/modules.d/50stratis
    sed 's|@LIBEXECDIR@|{{LIBEXECDIR}}|' dracut/50stratis/stratisd-min.service.in > {{DRACUTDIR}}/modules.d/50stratis/stratisd-min.service
    sed 's|@LIBEXECDIR@|{{LIBEXECDIR}}|' dracut/50stratis/module-setup.sh.in > {{DRACUTDIR}}/modules.d/50stratis/module-setup.sh
    {{INSTALL}} -Dpm0755 -t {{DRACUTDIR}}/modules.d/50stratis dracut/50stratis/stratis-rootfs-setup
    {{INSTALL}} -Dpm0644 -t {{DRACUTDIR}}/modules.d/50stratis dracut/50stratis/61-stratisd.rules
    {{INSTALL}} -Dpm0755 -d {{DRACUTDIR}}/modules.d/50stratis-clevis
    {{INSTALL}} -Dpm0755 -t {{DRACUTDIR}}/modules.d/50stratis-clevis dracut/50stratis-clevis/module-setup.sh
    {{INSTALL}} -Dpm0755 -t {{DRACUTDIR}}/modules.d/50stratis-clevis dracut/50stratis-clevis/stratis-clevis-rootfs-setup

# Install fstab script
install-fstab-script:
    mkdir -p {{UNITEXECDIR}}
    {{INSTALL}} -Dpm0755 -t {{UNITEXECDIR}} systemd/stratis-fstab-setup

# Install man pages
install-man-cfg:
    mkdir -p {{MANDIR}}/man8
    {{INSTALL}} -Dpm0644 -t {{MANDIR}}/man8 docs/stratisd.8
    {{INSTALL}} -Dpm0644 -t {{MANDIR}}/man8 docs/stratis-dumpmetadata.8

# Install all stratisd programs and config files
install-programs: install-udev-cfg install-dbus-cfg install-dracut-cfg install-systemd-cfg install-binaries install-udev-binaries install-fstab-script install-daemons

# Install systemd configuration
install-systemd-cfg:
    mkdir -p {{UNITDIR}}
    sed 's|@LIBEXECDIR@|{{LIBEXECDIR}}|' systemd/stratisd.service.in > {{UNITDIR}}/stratisd.service
    sed 's|@LIBEXECDIR@|{{LIBEXECDIR}}|' systemd/stratisd-min-postinitrd.service.in > {{UNITDIR}}/stratisd-min-postinitrd.service
    sed 's|@UNITEXECDIR@|{{UNITEXECDIR}}|' systemd/stratis-fstab-setup@.service.in > {{UNITDIR}}/stratis-fstab-setup@.service
    sed 's|@UNITEXECDIR@|{{UNITEXECDIR}}|' systemd/stratis-fstab-setup-with-network@.service.in > {{UNITDIR}}/stratis-fstab-setup-with-network@.service

# Install udev binaries
install-udev-binaries:
    mkdir -p {{UDEVDIR}}
    {{INSTALL}} -Dpm0755 -t {{UDEVDIR}} target/{{PROFILEDIR}}/stratis-base32-decode
    {{INSTALL}} -Dpm0755 -t {{UDEVDIR}} target/{{PROFILEDIR}}/stratis-str-cmp

# Install udev configuration
install-udev-cfg:
    mkdir -p {{UDEVDIR}}/rules.d
    {{INSTALL}} -Dpm0644 -t {{UDEVDIR}}/rules.d udev/61-stratisd.rules

# Build stratis-base32-decode binary
stratis-base32-decode:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{RUSTC}} {{RELEASE_FLAG}} \
    --bin=stratis-base32-decode \
    {{UDEV_FEATURES}} \
    {{TARGET_ARGS}} \
    -- {{STATIC_FLAG}}
    @ldd target/{{PROFILEDIR}}/stratis-base32-decode | grep --quiet --silent "statically linked" || (echo "stratis-base32-decode is not statically linked" && exit 1)

# Build stratis-min for early userspace
stratis-min:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratis-min {{MIN_FEATURES}} {{TARGET_ARGS}}

# Build stratis-str-cmp binary
stratis-str-cmp:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{RUSTC}} {{RELEASE_FLAG}} \
    --bin=stratis-str-cmp \
    {{UDEV_FEATURES}} \
    {{TARGET_ARGS}} \
    -- {{STATIC_FLAG}}
    @ldd target/{{PROFILEDIR}}/stratis-str-cmp | grep --quiet --silent "statically linked" || (echo "stratis-str-cmp is not statically linked" && exit 1)

# Build stratisd
stratisd:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratisd \
    {{TARGET_ARGS}}

# Build stratisd-min for early userspace
stratisd-min:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratisd-min {{SYSTEMD_FEATURES}} {{TARGET_ARGS}}

# Build the stratisd-tools program
stratisd-tools:
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="{{RUSTFLAGS}}" \
    cargo {{BUILD}} {{RELEASE_FLAG}} \
    --bin=stratisd-tools {{EXTRAS_FEATURES}} {{TARGET_ARGS}}

# Basic tests
test:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 cargo test --all-features -- --skip real_ --skip loop_ --skip clevis_ --skip test_stratis_min_ --skip test_stratisd_min_

# Clevis tests with loop devices
test-clevis-loop:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_loop_ -- --skip clevis_loop_should_fail_

# Clevis tests with loop devices as root
test-clevis-loop-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_loop_ -- --skip clevis_loop_should_fail_

# Clevis loop device tests that are expected to fail
test-clevis-loop-should-fail:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_loop_should_fail_

# Clevis loop device tests that are expected to fail as root
test-clevis-loop-should-fail-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_loop_should_fail_

# Clevis loop device tests that are expected to fail run under valgrind
test-clevis-loop-should-fail-valgrind:
    #!/usr/bin/env bash
    RUST_TEST_THREADS=1 sudo -E valgrind --leak-check=full --num-callers=500 $(cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "{{justfile_directory()}}/src/lib.rs") | select(.executable != null) | .executable') clevis_loop_should_fail_

# Clevis tests with loop devices with valgrind
test-clevis-loop-valgrind:
    #!/usr/bin/env bash
    RUST_TEST_THREADS=1 sudo -E valgrind --leak-check=full --num-callers=500 $(cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "{{justfile_directory()}}/src/lib.rs") | select(.executable != null) | .executable') clevis_loop_ --skip clevis_loop_should_fail_

# Clevis tests with real devices
test-clevis-real:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_real_ -- --skip clevis_real_should_fail

# Clevis tests with real devices as root
test-clevis-real-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_real_ -- --skip clevis_real_should_fail

# Clevis real device tests that are expected to fail
test-clevis-real-should-fail:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test clevis_real_should_fail

# Clevis real device tests that are expected to fail as root
test-clevis-real-should-fail-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test clevis_real_should_fail

# Verify that a script for comparing version specs with Fedora is available
test-compare-fedora-versions:
    @echo "Testing that COMPARE_FEDORA_VERSIONS environment variable is set to a valid path"
    test -e "{{COMPARE_FEDORA_VERSIONS}}"

# Tests with loop devices
test-loop:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test loop_ -- --skip clevis_loop_

# Tests with loop devices as root
test-loop-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test loop_ -- --skip clevis_loop_

# Tests run under valgrind with loop devices
test-loop-valgrind:
    #!/usr/bin/env bash
    RUST_TEST_THREADS=1 sudo -E valgrind --leak-check=full --num-callers=500 $(cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "{{justfile_directory()}}/src/lib.rs") | select(.executable != null) | .executable') loop_ --skip real_ --skip clevis_

# Tests with real devices
test-real:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test real_ -- --skip clevis_real_

# Tests with real devices as root
test-real-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test real_ -- --skip clevis_real_

# Test stratis-min CLI
test-stratis-min:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test --no-default-features --features "engine,min" test_stratis_min

# Test stratis-min CLI as root
test-stratis-min-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test --no-default-features --features "engine,min" test_stratis_min

# Test stratis-utils
test-stratis-utils:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test --no-default-features --features "dbus_enabled,engine" test_stratis_utils

# Test stratisd-min CLI
test-stratisd-min:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test --no-default-features --features "engine,min" test_stratisd_min

# Test stratisd-min CLI as root
test-stratisd-min-root:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test --no-default-features --features "engine,min" test_stratisd_min

# Test stratisd-tools CLI
test-stratisd-tools:
    RUSTFLAGS="{{RUSTFLAGS}}" RUST_BACKTRACE=1 RUST_TEST_THREADS=1 cargo test --no-default-features --features "engine,extras" test_stratisd_tools

# Basic tests run under valgrind
test-valgrind:
    #!/usr/bin/env bash
    RUST_TEST_THREADS=1 valgrind --leak-check=full --num-callers=500 $(cargo test --no-run --all-features --message-format=json 2>/dev/null | jq -r 'select(.target.src_path == "{{justfile_directory()}}/src/lib.rs") | select(.executable != null) | .executable') --skip real_ --skip loop_ --skip clevis_

# Run tmt lint
tmtlint:
    tmt lint

# Run yamllint on workflow files
yamllint:
    yamllint --strict .github/workflows/*.yml
    yamllint --strict .packit.yaml
    yamllint --strict .yamllint.yaml
