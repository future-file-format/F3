//! A very simple wasm lib to verify that wmemcheck will fail only calling `alloc`

/// Allocate memory.
///
/// # Safety
///
/// See [`std::alloc::GlobalAlloc::alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc(len: usize, align: usize) -> *mut u8 {
    std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(len, align))
}

/// Deallocate memory.
///
/// # Safety
///
/// See [`std::alloc::GlobalAlloc::dealloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize, align: usize) {
    std::alloc::dealloc(
        ptr,
        std::alloc::Layout::from_size_align_unchecked(len, align),
    );
}
