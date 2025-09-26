#!/bin/bash
# convert orc to dwrf and check memory usage
# used together with the Nimble repo
rm /mnt/nvme0n1/xinyu/laion/orc/merged_8M.dwrf
LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH ~/nimble/build/Release/fff-bench/mem_orc_to_dwrf /mnt/nvme0n1/xinyu/laion/orc/merged_8M.orc /mnt/nvme0n1/xinyu/laion/orc/merged_8M.dwrf > mem_orc.log