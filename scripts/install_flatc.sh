#!/bin/bash -e
git submodule update --init --recursive
cd third_party/flatbuffers
cmake -G "Unix Makefiles" -DCMAKE_BUILD_TYPE=Release
make -j flatc