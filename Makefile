.PHONY: all fmt check test

all: check

test:
	timeout 20s ./target/debug/evremap remap ./test.toml

check:
	cargo check

fmt:
	cargo +nightly fmt
