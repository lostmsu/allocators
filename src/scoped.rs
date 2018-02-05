//! A scoped linear allocator. This is something of a cross between a stack allocator
//! and a traditional linear allocator.

use std::cell::Cell;
use std::heap::{Alloc, AllocErr, Layout};
use std::mem;
use std::ptr;

use super::{Error, BlockOwner};

/// A scoped linear allocator.
pub struct Scoped<'parent, A: 'parent + Alloc> {
    allocator: &'parent mut A,
    current: Cell<*mut u8>,
    end: *mut u8,
    root: bool,
    start: *mut u8,
}

impl<'parent, A: Alloc> Scoped<'parent, A> {
    /// Creates a new `Scoped` backed by `size` bytes from the allocator supplied.
    pub fn new_from(alloc: &'parent mut A, size: usize) -> Result<Self, AllocErr> {
        // Create a memory buffer with the desired size and maximal align from the parent.
        let initial_block_layout = Layout::from_size_align(size, mem::align_of::<usize>()).unwrap();
        match unsafe { alloc.alloc(initial_block_layout.clone()) } {
            Ok(ptr) => Ok(Scoped {
                allocator: alloc,
                current: Cell::new(ptr),
                end: unsafe { ptr.offset(initial_block_layout.size() as isize) },
                root: true,
                start: ptr,
            }),
            Err(err) => Err(err),
        }
    }

    /// Calls the supplied function with a new scope of the allocator.
    ///
    /// Returns the result of the closure or an error if this allocator
    /// has already been scoped.
    pub fn scope<F, U>(&'parent mut self, f: F) -> Result<U, ()>
        where F: FnMut(&mut Self) -> U
    {
        if self.is_scoped() {
            return Err(());
        }

        let mut f = f;

        let old = self.current.get();
        let mut alloc = Scoped {
            allocator: self.allocator,
            current: self.current.clone(),
            end: self.end,
            root: false,
            start: old,
        };

        // set the current pointer to null as a flag to indicate
        // that this allocator is being scoped.
        self.current.set(ptr::null_mut());
        let u = f(&mut alloc);
        self.current.set(old);

        mem::forget(alloc);

        Ok(u)
    }

    // Whether this allocator is currently scoped.
    pub fn is_scoped(&self) -> bool {
        self.current.get().is_null()
    }
}

unsafe impl<'a, A: Alloc> Alloc for Scoped<'a, A> {
    unsafe fn alloc(&mut self, layout: Layout) -> Result<*mut u8, AllocErr> {
        if self.is_scoped() {
            return Err(AllocErr::invalid_input("Called allocate on already scoped \
                                                          allocator."
                                                             .into()));
        }

        if layout.size() == 0 {
            return Err(AllocErr::invalid_input("Can't allocate 0"));
        }

        let current_ptr = self.current.get();
        let aligned_ptr = super::align_forward(current_ptr, layout.align());
        let end_ptr = aligned_ptr.offset(layout.size() as isize);

        if end_ptr > self.end {
            Err(Error::out_of_memory(layout))
        } else {
            self.current.set(end_ptr);
            Ok(aligned_ptr)
        }
    }

    /// Because of the way this allocator is designed, reallocating a block that is not 
    /// the most recent will lead to fragmentation.
    unsafe fn realloc<'b>(&'b mut self, ptr: *mut u8, layout: Layout, new_layout: Layout)
                -> Result<*mut u8, AllocErr> {
        let current_ptr = self.current.get();

        if new_layout.size() == 0 {
            Err(AllocErr::invalid_input("Can't allocate 0"))
        } else if layout.align() == 0 {
            Err(Error::unsupported_alignment())
        } else if ptr.offset(layout.size() as isize) == current_ptr
               && new_layout.align() <= layout.align()  {
            // if this block is the last allocated, resize it if we can.
            // otherwise, we are out of memory.
            let new_cur = current_ptr.offset((new_layout.size() - layout.size()) as isize);
            if new_cur < self.end {
                self.current.set(new_cur);
                Ok(ptr)
            } else {
                Err(Error::out_of_memory(new_layout))
            }
        } else {
            // try to allocate a new block at the end, and copy the old mem over.
            // this will lead to some fragmentation.
            match self.alloc(new_layout) {
                Ok(new_ptr) => {
                    ptr::copy_nonoverlapping(ptr, new_ptr, layout.size());
                    Ok(new_ptr)
                }
                Err(err) => {
                    Err(err)
                }
            }
        }
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        if layout.size() == 0 || ptr.is_null() {
            return;
        }
        // no op for this unless this is the last allocation.
        // The memory gets reused when the scope is cleared.
        let current_ptr = self.current.get();
        if !self.is_scoped() && ptr.offset(layout.size() as isize) == current_ptr {
            self.current.set(ptr);
        }
    }
}

impl<'a, A: Alloc> BlockOwner for Scoped<'a, A> {
    fn owns_block(&self, ptr: *mut u8, _layout: Layout) -> bool {
        ptr >= self.start && ptr <= self.end
    }
}

impl<'a, A: Alloc> Drop for Scoped<'a, A> {
    /// Drops the `Scoped`
    fn drop(&mut self) {
        let size = self.end as usize - self.start as usize;
        // only free if this allocator is the root to make sure
        // that memory is freed after destructors for allocated objects
        // are called in case of unwind
        if self.root && size > 0 {
            let self_layout = Layout::from_size_align(size, mem::align_of::<usize>()).unwrap();
            unsafe { self.allocator.dealloc(self.start, self_layout) }
        }
    }
}

unsafe impl<'a, A: 'a + Alloc + Sync> Send for Scoped<'a, A> {}

#[cfg(test)]
mod tests {
    use super::super::*;
// TODO use_outer is now covered by borrow checker?
//    #[test]
//    #[should_panic]
//    fn use_outer() {
//        let alloc = &mut Scoped::new_from(&mut Heap::default(), 4).unwrap();
//        let mut outer_val = allocate(alloc, 0i32).unwrap();
//        alloc.scope(|_inner| {
//            // using outer allocator is dangerous and should fail.
//                 outer_val = allocate(alloc, 1i32).unwrap();
//             })
//             .unwrap();
//    }

    #[test]
    fn scope_scope() {
        let alloc = &mut Scoped::new_from(&mut Heap::default(), 64).unwrap();
        let _ = allocate(alloc, 0).unwrap();
        alloc.scope(|inner| {
                 let _ = allocate(inner, 32).unwrap();
                 inner.scope(|bottom| {
                          let _ = allocate(bottom, 23).unwrap();
                      })
                      .unwrap();
             })
             .unwrap();
    }

    #[test]
    fn out_of_memory() {
        // allocate more memory than the allocator has.
        let alloc = Scoped::new(0).unwrap();
        let (err, _) = alloc.allocate(1i32).err().unwrap();
        assert_eq!(err, AllocErr::OutOfMemory);
    }

    #[test]
    fn placement_in() {
        let alloc = Scoped::new(8_000_000).unwrap();
        // this would smash the stack otherwise.
        let _big = in alloc.make_place().unwrap() { [0u8; 8_000_000] };
    }

    #[test]
    fn owning() {
        let alloc = Scoped::new(64).unwrap();

        let val = alloc.allocate(1i32).unwrap();
        assert!(alloc.owns(&val));

        alloc.scope(|inner| {
                 let in_val = inner.allocate(2i32).unwrap();
                 assert!(inner.owns(&in_val));
                 assert!(!inner.owns(&val));
             })
             .unwrap();
    }

    #[test]
    fn mutex_sharing() {
        use std::thread;
        use std::sync::{Arc, Mutex};
        let alloc = Scoped::new_from(&mut Heap::default(), 64).unwrap();
        let data = Arc::new(Mutex::new(alloc));
        for i in 0..10 {
            let data = data.clone();
            thread::spawn(move || {
                let alloc_handle = data.lock().unwrap();
                let _ = alloc_handle.allocate(i).unwrap();
            });
        }
    }
}
