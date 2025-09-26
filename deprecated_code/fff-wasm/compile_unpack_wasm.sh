#!/bin/bash 
set -e
emcc -std=c++17 -O3 --no-entry -msimd128 -sWASM=1 \
-sEXPORTED_FUNCTIONS='["_pack_8bit_32ow","_unpack_8bw_32ow_32crw_1uf","_alloc","_dealloc","_fls_untranspose_generated","_unrolled_RLE_decoding"]' \
fallback_scalar_aav_1024_uf1_unpack_src.cpp -o wasm-generated/fls_example.wasm
# add -fno-rtti -fno-exceptions -flto not working
# add -msimd128 improves a lot ~ 3.5 times. reasonable
# wasm2wat unpack.wasm -o unpack.wat  
# cp unpack.wasm ../fff-bench/test_wasm/