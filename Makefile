.PHONY: all run test check build-release clean docker-build docker-up docker-down docker-logs

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

docker-build:
	docker build -t okx-2pa-agent:latest .

docker-up:
	docker compose up -d

docker-down:
	docker compose down

docker-logs:
	docker compose logs -f
