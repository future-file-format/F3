<p align="center"> <img src="doc/f3-logo.svg" alt="f3_logo" width="200"/> </p>

# F3: The Open-Source Data File Format for the Future

F3 is a data file format that is designed with efficiency, interoperability, and extensibility in mind. It provides a data organization that rectifies the layout shortcomings of the last-generation formats like Parquet, while at the same time maintaining good interoperability and extensibility (a.k.a future-proof) via embedded Wasm decoders.

> ⚠️ This project is a research prototype verifying the ideas in the paper. You should not use it in production.

## Build instructions

We only tested on an Intel machine with Debian 12.

```shell
git submodule update --init --recursive
./scripts/setup_debian.sh
# build the PoC package of F3
cargo build -p fff-poc
# run unit test for F3
cargo test -p fff-poc
```

## Important directories

[format](format): FlatBuffer definition of the file format.

[fff-poc](fff-poc): The main code of the F3 format. It references other subdirs like fff-core, fff-encoding, fff-format, and fff-ude-wasm.

[fff-bench](fff-bench): Benchmarks and experiments appeared in the paper. Specifically, [fff-bench/examples](fff-bench/examples) should contain most experiments, both micro and e2e.

fff-ude*: ude stand for User-Defined-Encoding and code in those directories relates to the Wasm decoding implementation.

[scripts](scripts) and [exp_scripts](exp_scripts): scripts related to run the experiments.

## Reproduction steps for the experiment results in the paper

Please refer to [doc/paper_reproduction.md](doc/paper_reproduction.md) for the detailed steps.

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Citation

If you find this project useful, please consider citing:

```bibtex
@article{zeng2025f3,
author = {Zeng, Xinyu and Meng, Ruijun and Prammer, Martin and McKinney, Wes and Patel, Jignesh M. and Pavlo, Andrew and Zhang, Huanchen},
title = {F3: The Open-Source Data File Format for the Future},
year = {2025},
issue_date = {September 2025},
publisher = {Association for Computing Machinery},
address = {New York, NY, USA},
volume = {3},
number = {4},
url = {https://doi.org/10.1145/3749163},
doi = {10.1145/3749163},
abstract = {Columnar storage formats are the foundation for modern data analytics systems. The proliferation of open-source file formats (i.e., Parquet, ORC) allows seamless data sharing across disparate platforms. However, these formats were created over a decade ago for hardware and workload environments that are much different from today. Although these formats have incorporated some updates to their specification to adapt to these changes, not all deployments support those modifications, and too often systems cannot overcome the formats' deficiencies and limitations without a rewrite.In this paper, we present the Future-proof File Format (F3) project. It is a next-generation open-source file format with interoperability, extensibility, and efficiency as its core design principles. F3 obviates the need to create a new format every time a shift occurs in data processing and computing by providing a data organization structure and a general-purpose API to allow developers to add new encoding schemes easily. Each self-describing F3 file includes both the data and meta-data, as well as WebAssembly (Wasm) binaries to decode the data. Embedding the decoders in each file requires minimal storage (kilobytes) and ensures compatibility on any platform in case native decoders are unavailable. To evaluate F3, we compared it against legacy and state-of-the-art open-source file formats. Our evaluations demonstrate the efficacy of F3's storage layout and the benefits of Wasm-driven decoding.},
journal = {Proc. ACM Manag. Data},
month = sep,
articleno = {245},
numpages = {27},
keywords = {columnar storage, compression, extensibility, file format}
}
```
