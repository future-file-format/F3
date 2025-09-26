#!/bin/bash
cargo run --example wasm_decode_micro_exp --release 2>/dev/null | tee results/wasm_micro.csv