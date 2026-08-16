#!/usr/bin/env bash
set -e

echo "===================================================="
echo "  OKX 2PA Agent (Rust High-Performance Edition)"
echo "  Starting server at http://127.0.0.1:8088 ..."
echo "===================================================="

if [ -f "./target/release/okx-2pa-agent" ]; then
    ./target/release/okx-2pa-agent --host 127.0.0.1 --port 8088
else
    cargo run --release -- --host 127.0.0.1 --port 8088
fi
