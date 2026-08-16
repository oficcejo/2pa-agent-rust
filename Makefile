.PHONY: all run test check build-release clean

all: check test

run:
	cargo run -- --host 127.0.0.1 --port 8088

test:
	cargo test -- --nocapture

check:
	cargo check

build-release:
	cargo build --release

clean:
	cargo clean
