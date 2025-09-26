#!/bin/bash -e
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
sudo apt install build-essential cmake git python3 libpython3-dev

sudo apt satisfy "protobuf-compiler (>= 3.15.0)"
# If the command above fails, install pre-compiled protoc binaries
# See instruction in https://protobuf.dev/installation/
# PB_REL="https://github.com/protocolbuffers/protobuf/releases"
# cd /tmp
# curl -LO $PB_REL/download/v30.2/protoc-30.2-linux-x86_64.zip
# unzip protoc-30.2-linux-x86_64.zip -d $HOME/.local
# echo "export PATH=\$PATH:\$HOME/.local/bin" >> ~/.profile

sudo apt install gcc-multilib # to compile 32-bit binaries for WASM
# emscripten
mkdir -p ~/.local
cd ~/.local
git clone https://github.com/emscripten-core/emsdk.git
cd emsdk
./emsdk install latest
./emsdk activate latest
echo "source \$HOME/.local/emsdk/emsdk_env.sh" >> ~/.profile
# for wasm-opt
wget https://github.com/WebAssembly/binaryen/releases/download/version_120_b/binaryen-version_120_b-x86_64-linux.tar.gz
tar -xvf binaryen-version_120_b-x86_64-linux.tar.gz
echo "export PATH=\$PATH:\$HOME/.local/binaryen-version_120_b/bin" >> ~/.profile
