.PHONY: fmt fmt-check clippy test doc check build clean

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all-features

doc:
	cargo doc --no-deps --all-features

check: fmt-check clippy test

build:
	cargo build --release --all-features

clean:
	cargo clean
