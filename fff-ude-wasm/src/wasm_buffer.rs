use std::{
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use arrow_buffer::Buffer;

use crate::Instance;

/// A Buffer representation that allocates memory in WASM memory space.
/// It has a custom drop that will calls to WASM.
pub struct WasmBuffer {
    /// usize is Send+Sync while *const u8 is not.
    host_ptr: usize,
    _guest_ptr: u32,
    len: u32,
    arrow_buffer_address: u32,
    instance: Arc<Mutex<Instance>>,
}

impl WasmBuffer {
    pub fn new(
        host_ptr: usize,
        guest_ptr: u32,
        len: u32,
        arrow_buffer_address: u32,
        instance: Arc<Mutex<Instance>>,
    ) -> Self {
        Self {
            host_ptr,
            _guest_ptr: guest_ptr,
            len,
            arrow_buffer_address,
            instance,
        }
    }
}

impl Drop for WasmBuffer {
    fn drop(&mut self) {
        // println!("{:#x}", self.host_ptr);
        // println!("{:#x}", self.guest_ptr);
        // dbg!(self.len);
        // BUGFIX(1021): only dealloc when the buffer is not dangling ptr
        // if self.len() > 0 {
        let mut instance = self.instance.lock().unwrap();
        // BUGFIX(1223): we should drop the Box::<arrow_buffer::Buffer> in Wasm, not dealloc the bytes
        instance.buffer_drop(self.arrow_buffer_address).unwrap();
        // instance.print_stdio();
        // }
    }
}

impl AsRef<[u8]> for WasmBuffer {
    fn as_ref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.host_ptr as *const u8, self.len as usize) }
    }
}

impl Deref for WasmBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl DerefMut for WasmBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::slice::from_raw_parts_mut(self.host_ptr as *mut u8, self.len as usize) }
    }
}

impl From<WasmBuffer> for Buffer {
    fn from(mut wasm_buffer: WasmBuffer) -> Self {
        unsafe {
            Buffer::from_custom_allocation(
                // FIXME: return raw pointer is highly unsafe. Any subsequent call to grow/release the WASM memory will
                // make this pointer invalid.
                // See https://docs.rs/wasmtime/24.0.0/wasmtime/struct.Memory.html
                NonNull::new_unchecked(wasm_buffer.as_mut_ptr()),
                wasm_buffer.len(),
                Arc::new(wasm_buffer),
            )
        }
    }
}
