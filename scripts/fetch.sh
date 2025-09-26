#!/bin/bash
# This script is used to fetch zig dependencies in a restricted network environment (e.g., China).
# 
# Example error encountered:
#   error: the following build command failed with exit code 1:
#  /Users/xinyu/.cargo/git/checkouts/vortex-fa765f25da8a1568/2947879/deps/fastlanez/zig-cache/o/ce42a9c2c75a017ee958c6df4eb3b413/build /opt/homebrew/Cellar/zig/0.12.0/bin/zig /Users/xinyu/.cargo/git/checkouts/vortex-fa765f25da8a1568/2947879/deps/fastlanez /Users/xinyu/.cargo/git/checkouts/vortex-fa765f25da8a1568/2947879/deps/fastlanez/zig-cache /Users/xinyu/.cache/zig --seed 0x85241e29 -Z4152e2f642b613e6 lib -Doptimize=Debug --summary all
#  thread 'main' panicked at /Users/xinyu/.cargo/git/checkouts/vortex-fa765f25da8a1568/2947879/fastlanez-sys/build.rs:44:9:
#  failed to successfully invoke `zig build` in /Users/xinyu/.cargo/git/checkouts/vortex-fa765f25da8a1568/2947879
#
# Then you should do:
# ```
# cd /Users/xinyu/.cargo/git/checkouts/vortex-fa765f25da8a1568/2947879/deps/fastlanez/
# bash $PATH_TO_THIS_SCRIPT/fetch.sh
# ```
set -e

if [ $# -ne 1 ] && [ ! -e build.zig.zon ]; then
  echo "Couldn't find build.zig.zon file, please give path to it, or change current dir to a decent zig project"
  echo "  usage: zfetch.sh [build.zig.zon]"
  exit -1
fi

do_fetch() {
  for d in `grep -o 'https://.*tar\.gz' $1`; do
    wget $d
    tarfile=${d##*/}
    hash=`zig fetch --debug-hash $tarfile | tail -n 1`
    rm $tarfile
    if [ -e ~/.cache/zig/p/$hash/build.zig.zon ]; then
      do_fetch ~/.cache/zig/p/$hash/build.zig.zon
    fi
  done
}

zonfile=$1
if [ -z "$1" ]; then
  zonfile=build.zig.zon
fi

do_fetch $zonfile
