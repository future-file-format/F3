use std::ffi::CString;
use std::os::raw::c_char;

/// Allocate memory.
///
/// # Safety
///
/// See [`std::alloc::GlobalAlloc::alloc`].
#[no_mangle]
pub unsafe extern "C" fn alloc(len: usize, align: usize) -> *mut u8 {
    std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(len, align))
}

/// Deallocate memory.
///
/// # Safety
///
/// See [`std::alloc::GlobalAlloc::dealloc`].
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize, align: usize) {
    std::alloc::dealloc(
        ptr,
        std::alloc::Layout::from_size_align_unchecked(len, align),
    );
}

#[no_mangle]
pub extern "C" fn create_wasm_memory(len: usize) -> *const u8 {
    // Allocating memory for the string (+1 for null terminator)
    let ptr = unsafe {
        // Using Vec<u8> to allocate memory in the WASM heap
        let mut buffer = Vec::with_capacity(len);
        buffer.set_len(len);

        // Directly write the string into the vector's buffer
        // buffer[..len].copy_from_slice(bytes);

        // Add a null terminator (optional, based on how you want to use the pointer in JavaScript)
        // buffer[len] = 0;

        // Returning the pointer to the string in WASM memory
        let ptr = buffer.as_ptr();
        // Prevent the buffer from being deallocated at the end of the scope,
        // This passes ownership of the memory to the caller.
        std::mem::forget(buffer);

        ptr
    };

    // return the raw pointer
    ptr
}

#[no_mangle]
pub extern "C" fn deallocate_str(ptr: *mut u8, len: usize) {
    // Create a Vec from the parts to have it deallocated
    unsafe {
        Vec::from_raw_parts(ptr, len, len);
    };
}

#[no_mangle]
pub unsafe extern "C" fn my_greet_ptr(name: *const c_char) -> *const c_char {
    // Convert the given name from C string to a Rust string
    let name = std::ffi::CStr::from_ptr(name);
    let name_str = name.to_str().unwrap();

    // Create the greet string
    let greet_str = format!("Hello, {}!", name_str);
    let greet_cstr = CString::new(greet_str).unwrap(); // CString to handle strings with null byte

    // Get the pointer to the raw C string and forget it to prevent it from being automatically deallocated
    let ptr = greet_cstr.into_raw();

    // Return both the pointer
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn my_greet_len(ptr: *const c_char) -> usize {
    // Convert the given pointer to a CString
    let greet_cstr = CString::from_raw(ptr as *mut c_char);

    // Get the length of the string
    let len = greet_cstr.as_bytes().len();

    // Forget the CString to prevent it from being automatically deallocated
    std::mem::forget(greet_cstr);

    // Return the length
    len
}

#[no_mangle]
pub extern "C" fn free_greet_string(ptr: *mut c_char) {
    unsafe {
        // Retake the pointer to drop the CString, allowing it to clean up properly
        if !ptr.is_null() {
            let _ = CString::from_raw(ptr);
        }
    }
}
