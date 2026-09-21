//! Offline probe for feature `video-reference-media-v1`.

#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

pub mod caps;
pub mod config;

#[cfg(any(target_arch = "wasm32", test))]
mod wasm_mem;

#[cfg(target_arch = "wasm32")]
#[path = "guest.rs"]
mod guest;
