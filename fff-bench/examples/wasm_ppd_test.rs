use std::sync::{Arc, Mutex};

use arrow_array::{ArrayRef, Int32Array};
use fff_core::util::buffer_to_array::primitive_array_from_arrow_buffers_iter;
use fff_ude_wasm::{Config, Instance, Runtime};
use rand::Rng;
use wasm_test_encoders::encode_fff_general;
use wasmtime::Engine;

/// Generated using Claude
fn gen_dict_data() -> Vec<i32> {
    // Dictionary size - limiting to a small set of values
    const DICT_SIZE: usize = 12;
    // Total data size
    const DATA_SIZE: usize = 65536;

    let mut rng = rand::thread_rng();

    // Create a dictionary of values with varying bit widths
    let dictionary: Vec<i32> = vec![
        1000000, // Large value replacing -10000000
        8000000, 654321, 22, 0, 7, // Small values would be efficient with bitpacking
        42, 123, 1000, 123456, // Medium values
        5000000, 10000000, // Large values
    ];

    // Create a probability distribution that favors certain values
    // This creates data that's skewed toward certain dictionary values
    let mut probabilities = vec![
        0.02, 0.02, 0.03, 0.05, 0.05, 0.25, 0.20, 0.15, 0.10, 0.08, 0.03, 0.02,
    ];

    // Ensure probabilities sum to 1.0
    let sum: f64 = probabilities.iter().sum();
    for p in &mut probabilities {
        *p /= sum;
    }

    // Generate data according to the probability distribution
    let mut data = Vec::with_capacity(DATA_SIZE);

    // Create cumulative distribution for sampling
    let mut cumulative = vec![0.0; DICT_SIZE];
    cumulative[0] = probabilities[0];
    for i in 1..DICT_SIZE {
        cumulative[i] = cumulative[i - 1] + probabilities[i];
    }

    // Generate the data
    for _ in 0..DATA_SIZE {
        let r: f64 = rng.gen();
        let idx = cumulative
            .iter()
            .position(|&x| r <= x)
            .unwrap_or(DICT_SIZE - 1);
        data.push(dictionary[idx]);
    }
    data
}

fn main() {
    let engine =
        Engine::new(wasmtime::Config::new().profiler(wasmtime::ProfilingStrategy::None)).unwrap();
    let rt = Arc::new(
        Runtime::with_config_engine(
            &std::fs::read(
                "/home/xinyu/fff-devel/target/wasm32-wasip1/release/adv_ude_fff.wasm",
                // "/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/adv_ude_fff.wasm",
            )
            .unwrap(),
            Config::default(),
            &engine,
        )
        .unwrap(),
    );
    let instance = Arc::new(Mutex::new(Instance::new(&rt).unwrap()));
    let mut guard = instance.lock().unwrap();

    let int_data: Vec<i32> = gen_dict_data();
    let array = Arc::new(Int32Array::from(int_data.clone())) as ArrayRef;
    let encoded = encode_fff_general(array.clone());
    let ppd_expr = fff_ude::kwargs::ppd_serialize(fff_ude::kwargs::PPDExpr::new(
        fff_ude::kwargs::Operator::Eq,
        fff_ude::kwargs::ScalarValue::I32(0),
    ));
    let kwargs = vec![("ppd".as_bytes(), ppd_expr.as_slice())];
    let kwargs_ser = fff_ude::kwargs::kwargs_serialize(kwargs.as_slice());
    let slice = guard.call_init(&encoded, &kwargs_ser).unwrap();
    let iter = guard
        .call_decode(slice.ptr(), instance.clone())
        .unwrap()
        .unwrap();
    drop(guard);
    let out = primitive_array_from_arrow_buffers_iter(
        &arrow::datatypes::DataType::Boolean,
        iter,
        int_data.len() as u64,
    )
    .unwrap();

    dbg!(array);
    dbg!(out);
    // assert_eq!(*array, *out);
}
