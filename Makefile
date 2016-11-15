check: fmt

${HOME}/.cargo/bin/cargo-fmt:
	cargo install rustfmt

fmt: ${HOME}/.cargo/bin/cargo-fmt
	PATH=${HOME}/.cargo/bin:${PATH} cargo fmt -- --write-mode=diff
