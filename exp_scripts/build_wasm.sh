#!/bin/bash

# Determine the project root directory dynamically
PROJECT_ROOT=$(git rev-parse --show-toplevel)

schemes=("pco" "lz4" "flsbp" "fff" "gzip" "zstd")

# echo -n "${schemes[@]}" | tr ' ' ','
echo -n "scheme"
echo -n ",size"
echo -n ",comp_time"
echo -n ",target"
echo -n ",profile"
echo ",compr,i"

targets=("wasm32-wasip1")
# targets=("wasm32-wasip1" "wasm32-unknown-unknown")
profiles=("release" "opt-size" "opt-size-lvl3")
comprs=("none" "zstd")

for target in "${targets[@]}"; do
    for profile in "${profiles[@]}"; do
        for scheme in "${schemes[@]}"; do
            for compr in "${comprs[@]}"; do
                for i in {1..5}; do
                    # echo "Building scheme: $scheme"
                    cd "$PROJECT_ROOT/wasm-libs/fff-ude-example-$scheme" || exit
                    if [ "$profile" == "opt-size" ] || [ "$profile" == "opt-size-lvl3" ]; then
                        extra_flags="-Z build-std=std,panic_abort -Z build-std-features=panic_immediate_abort"
                    else
                        extra_flags=""
                    fi
                    cargo build --target "$target" --profile $profile $extra_flags 2>/dev/null

                    cd "$PROJECT_ROOT/target/$target/$profile" || exit
                    
                    if [ "$compr" == "none" ]; then
                        file_size=$(stat -c%s "fff_ude_example_$scheme.wasm")
                        start_time=$(date +%s.%N)
                        wasmtime compile "fff_ude_example_$scheme.wasm" >/dev/null 2>&1
                        comp_time=$(echo "$(date +%s.%N) - $start_time" | bc)
                    else
                        zstd -q -f "fff_ude_example_$scheme.wasm"
                        file_size=$(stat -c%s "fff_ude_example_$scheme.wasm.zst")
                        start_time=$(date +%s.%N)
                        zstd -q -d -f "fff_ude_example_$scheme.wasm.zst" >/dev/null 2>&1
                        wasmtime compile "fff_ude_example_$scheme.wasm" >/dev/null 2>&1
                        comp_time=$(echo "$(date +%s.%N) - $start_time" | bc)
                    fi

                    echo "${scheme},${file_size},${comp_time},${target},${profile},${compr},${i}"
                done
            done
        done
        
    done
done
