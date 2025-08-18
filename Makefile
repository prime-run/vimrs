.PHONY: all fmt check clippy test dev

all: check

test:
	cargo test

check:
	cargo check

fmt:
	cargo fmt

clippy:
	cargo clippy --fix --allow-dirty

dev: check
	cargo build --out-dir ./bin -Z unstable-options

clean:
	cargo clean
	rm -rf ./bin
