//! This module contains some composable building blocks to build allocator chains.

use std::heap::{Alloc, AllocErr, Layout};
use super::{Error, BlockOwner};

/// This allocator always fails.
/// It will panic if you try to deallocate with it.
pub struct NullAllocator;

unsafe impl Alloc for NullAllocator {
    unsafe fn alloc(&mut self, layout: Layout) -> Result<*mut u8, AllocErr> {
        Err(Error::out_of_memory(layout))
    }

    unsafe fn realloc<'a>(&'a mut self,
                          ptr: *mut u8,
                          layout: Layout,
                          new_layout: Layout) -> Result<*mut u8, AllocErr> {
        Err(AllocErr::Exhausted{request: new_layout})
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        panic!("Attempted to deallocate using null allocator.")
    }
}

impl BlockOwner for NullAllocator {
    fn owns_block(&self, ptr: *mut u8, layout: Layout) -> bool {
        false
    }
}

/// This allocator has a main and a fallback allocator.
/// It will always attempt to allocate first with the main allocator,
/// and second with the fallback.
pub struct Fallback<M: BlockOwner, F: BlockOwner> {
    main: M,
    fallback: F,
}

impl<M: BlockOwner, F: BlockOwner> Fallback<M, F> {
    /// Create a new `Fallback`
    pub fn new(main: M, fallback: F) -> Self {
        Fallback {
            main: main,
            fallback: fallback,
        }
    }
}

unsafe impl<M: BlockOwner, F: BlockOwner> Alloc for Fallback<M, F> {
    unsafe fn alloc(&mut self, layout: Layout) -> Result<*mut u8, AllocErr> {
        match self.main.alloc(layout) {
            Ok(ptr) => Ok(ptr),
            Err(_) => self.fallback.alloc(layout),
        }
    }

    unsafe fn realloc<'a>(&'a mut self, ptr: *mut u8,
                          layout: Layout,
                          new_layout: Layout) -> Result<*mut u8, AllocErr> {
        if self.main.owns_block(ptr, layout) {
            self.main.realloc(ptr, layout, new_layout)
        } else if self.fallback.owns_block(ptr, layout) {
            self.fallback.realloc(ptr, layout, new_layout)
        } else {
            Err(AllocErr::invalid_input("Neither fallback nor main owns this block.".into()))
        }
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        if self.main.owns_block(ptr, layout) {
            self.main.dealloc(ptr, layout);
        } else if self.fallback.owns_block(ptr, layout) {
            self.fallback.dealloc(ptr, layout);
        }
    }
}

impl<M: BlockOwner, F: BlockOwner> BlockOwner for Fallback<M, F> {
    fn owns_block(&self, ptr: *mut u8, layout: Layout) -> bool {
        self.main.owns_block(ptr, layout) || self.fallback.owns_block(ptr, layout)
    }
}

/// Something that logs an allocator's activity.
/// In practice, this may be an output stream,
/// a data collector, or seomthing else entirely.
pub trait ProxyLogger {
    /// Called after a successful allocation.
    fn allocate_success(&self, ptr: *mut u8, layout: Layout);
    /// Called after a failed allocation.
    fn allocate_fail(&self, err: &AllocErr, layout: Layout);

    /// Called when deallocating a block.
    fn deallocate(&self, ptr: *mut u8, layout: Layout);

    /// Called after a successful reallocation.
    fn reallocate_success(&self, old_ptr: *mut u8, old_layout: Layout, new_ptr: *mut u8, new_layout: Layout);
    /// Called after a failed reallocation.
    fn reallocate_fail(&self, err: &AllocErr, ptr: *mut u8, layout: Layout, req_size: usize);
}

/// This wraps an allocator and a logger, logging all allocations
/// and deallocations.
pub struct Proxy<A, L> {
    alloc: A,
    logger: L,
}

impl<A: Alloc, L: ProxyLogger> Proxy<A, L> {
    /// Create a new proxy allocator.
    pub fn new(alloc: A, logger: L) -> Self {
        Proxy {
            alloc: alloc,
            logger: logger,
        }
    }
}

unsafe impl<A: Alloc, L: ProxyLogger> Alloc for Proxy<A, L> {
    unsafe fn alloc(&mut self, layout: Layout) -> Result<*mut u8, AllocErr> {
        match self.alloc.alloc(layout) {
            Ok(ptr) => {
                self.logger.allocate_success(ptr, layout);
                Ok(ptr)
            }
            Err(err) => {
                self.logger.allocate_fail(&err, layout);
                Err(err)
            }
        }
    }

    unsafe fn realloc<'a>(&'a mut self, ptr: *mut u8,
                          layout: Layout,
                          new_layout: Layout) -> Result<*mut u8, AllocErr> {
        match self.alloc.realloc(ptr, layout, new_layout) {
            Ok(new_ptr) => {
                self.logger.reallocate_success(ptr, layout, new_ptr, new_layout);
                Ok(new_ptr)
            }
            Err(err) => {
                self.logger.reallocate_fail(&err, ptr, layout, new_layout.size());
                Err(err)
            }
        }
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        self.logger.deallocate(ptr, layout);
        self.alloc.dealloc(ptr, layout);
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    #[should_panic]
    fn null_allocate() {
        let alloc = NullAllocator;
        alloc.allocate(1i32).unwrap();
    }
}
