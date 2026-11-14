//! Buffers in linear memory, the memory the GPU and the DSP can read.

use core::alloc::Layout;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use citronnade_rt::heap;

use crate::{Result, OUT_OF_MEMORY};

/// A fixed-size array of `T` in linear memory (128-byte aligned), for
/// vertices, indices and textures.
pub struct LinearBuffer<T: Copy> {
    ptr: NonNull<T>,
    len: usize,
    _marker: PhantomData<T>,
}

unsafe impl<T: Copy + Send> Send for LinearBuffer<T> {}
unsafe impl<T: Copy + Sync> Sync for LinearBuffer<T> {}

impl<T: Copy> LinearBuffer<T> {
    fn layout(len: usize) -> Layout {
        let size = (len * size_of::<T>()).max(1);
        Layout::from_size_align(size, align_of::<T>().max(0x80)).expect("buffer too large")
    }

    /// A buffer of `len` zeroed elements.
    pub fn zeroed(len: usize) -> Result<LinearBuffer<T>> {
        let ptr = heap::linear_alloc(Self::layout(len)).ok_or(OUT_OF_MEMORY)?;
        unsafe { ptr.as_ptr().write_bytes(0, Self::layout(len).size()) };
        Ok(LinearBuffer { ptr: ptr.cast(), len, _marker: PhantomData })
    }

    /// A buffer holding a copy of `data`.
    pub fn from_slice(data: &[T]) -> Result<LinearBuffer<T>> {
        let ptr = heap::linear_alloc(Self::layout(data.len())).ok_or(OUT_OF_MEMORY)?;
        let ptr = ptr.cast::<T>();
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), ptr.as_ptr(), data.len()) };
        Ok(LinearBuffer { ptr, len: data.len(), _marker: PhantomData })
    }

    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// Size in bytes.
    pub fn byte_len(&self) -> usize {
        self.len * size_of::<T>()
    }

    /// The physical address the GPU uses.
    pub fn phys(&self) -> u32 {
        crate::phys(self.ptr.as_ptr().cast())
    }
}

impl<T: Copy> Deref for LinearBuffer<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<T: Copy> DerefMut for LinearBuffer<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl<T: Copy> Drop for LinearBuffer<T> {
    fn drop(&mut self) {
        unsafe { heap::linear_free(self.ptr.cast(), Self::layout(self.len)) };
    }
}
