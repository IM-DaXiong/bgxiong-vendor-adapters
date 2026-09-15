//! C mem* for no_std wasm (wasm-component-ld rejects `env::memcmp` without these).
//!
//! Implementations are explicit byte loops. Slice ordering and bulk-memory
//! intrinsics lower to these same symbols on wasm32 and recurse.

extern crate alloc;

/// Byte-wise memcmp. Must not call slice ordering (that lowers to this symbol).
pub unsafe fn memcmp_bytes(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b {
            return (a as i32) - (b as i32);
        }
        i += 1;
    }
    0
}

pub unsafe fn memcpy_bytes(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dest.add(i) = *src.add(i);
        i += 1;
    }
    dest
}

pub unsafe fn memmove_bytes(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if n == 0 || dest as usize == src as usize {
        return dest;
    }
    if (dest as usize) < (src as usize) {
        let mut i = 0;
        while i < n {
            *dest.add(i) = *src.add(i);
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *dest.add(i) = *src.add(i);
        }
    }
    dest
}

pub unsafe fn memset_bytes(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    let b = c as u8;
    let mut i = 0;
    while i < n {
        *s.add(i) = b;
        i += 1;
    }
    s
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    memcmp_bytes(s1, s2, n)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    memcpy_bytes(dest, src, n)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    memmove_bytes(dest, src, n)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    memset_bytes(s, c, n)
}

/// wit-bindgen 0.51 only ships cabi_realloc when target_env != p2.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: *mut u8,
    old_len: usize,
    align: usize,
    new_len: usize,
) -> *mut u8 {
    use alloc::alloc::{alloc, handle_alloc_error, realloc, Layout};
    let layout;
    let ptr = if old_len == 0 {
        if new_len == 0 {
            return align as *mut u8;
        }
        layout = Layout::from_size_align_unchecked(new_len, align);
        alloc(layout)
    } else {
        layout = Layout::from_size_align_unchecked(old_len, align);
        realloc(old_ptr, layout, new_len)
    };
    if ptr.is_null() {
        if cfg!(debug_assertions) {
            handle_alloc_error(layout);
        } else {
            core::arch::wasm32::unreachable();
        }
    }
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memcmp_equal_and_zero_len() {
        let a = [1u8, 2, 3];
        let b = [1u8, 2, 3];
        unsafe {
            assert_eq!(memcmp_bytes(a.as_ptr(), b.as_ptr(), 3), 0);
            assert_eq!(memcmp_bytes(a.as_ptr(), b.as_ptr(), 0), 0);
        }
    }

    #[test]
    fn memcmp_unequal_and_shared_prefix() {
        let a = [1u8, 2, 9];
        let b = [1u8, 2, 3];
        unsafe {
            assert!(memcmp_bytes(a.as_ptr(), b.as_ptr(), 3) > 0);
            assert_eq!(memcmp_bytes(a.as_ptr(), b.as_ptr(), 2), 0);
        }
    }

    #[test]
    fn memcpy_and_memset() {
        let src = [9u8, 8, 7, 6];
        let mut dest = [0u8; 4];
        unsafe {
            memcpy_bytes(dest.as_mut_ptr(), src.as_ptr(), 4);
            assert_eq!(dest, src);
            memset_bytes(dest.as_mut_ptr(), 0xAB, 4);
            assert_eq!(dest, [0xAB; 4]);
            memset_bytes(dest.as_mut_ptr(), 0, 0);
            assert_eq!(dest, [0xAB; 4]);
        }
    }

    #[test]
    fn memmove_overlap_forward_and_back() {
        let mut buf = [1u8, 2, 3, 4, 5];
        unsafe {
            memmove_bytes(buf.as_mut_ptr().add(2), buf.as_ptr(), 3);
            assert_eq!(buf, [1, 2, 1, 2, 3]);
        }
        let mut buf = [1u8, 2, 3, 4, 5];
        unsafe {
            memmove_bytes(buf.as_mut_ptr(), buf.as_ptr().add(2), 3);
            assert_eq!(buf, [3, 4, 5, 4, 5]);
        }
    }
}
