use fff_ude::ffi::general_wrapper;
use wasm_test_encoders::decode_fff_general;

// use talc::*;

// static mut ARENA: [u8; 10000] = [0; 10000];

// #[global_allocator]
// static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
//     // if we're in a hosted environment, the Rust runtime may allocate before
//     // main() is called, so we need to initialize the arena automatically
//     ClaimOnOom::new(Span::from_array(core::ptr::addr_of!(ARENA).cast_mut()))
// })
// .lock();
// static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[no_mangle]
pub unsafe extern "C" fn decode_general_ffi(
    ptr: *const u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    general_wrapper(decode_fff_general, ptr, len, out)
}
