//! Custom memory allocators and utilities for using them.
//!
//! # Examples
//! ```rust
//! #![feature(placement_in_syntax)]
//!
//! use std::io;
//! use allocators::{Allocator, Scoped, BlockOwner, FreeList, Proxy};
//!
//! #[derive(Debug)]
//! struct Bomb(u8);
//!
//! impl Drop for Bomb {
//!     fn drop(&mut self) {
//!         println!("Boom! {}", self.0);
//!     }
//! }
//! // new scoped allocator with 4 kilobytes of memory.
//! let alloc = Scoped::new(4 * 1024).unwrap();
//!
//! alloc.scope(|inner| {
//!     let mut bombs = Vec::new();
//!     // allocate makes the value on the stack first.
//!     for i in 0..100 { bombs.push(inner.allocate(Bomb(i)).unwrap())}
//!     // there's also in-place allocation!
//!     let bomb_101 = in inner.make_place().unwrap() { Bomb(101) };
//!     // watch the bombs go off!
//! });
//!
//!
//! // You can make allocators backed by other allocators.
//! {
//!     let secondary_alloc = FreeList::new_from(&alloc, 128, 8).unwrap();
//!     let mut val = secondary_alloc.allocate(0i32).unwrap();
//!     *val = 1;
//! }
//!
//! ```

#![feature(
    allocator_api,
    coerce_unsized,
    heap_api,
    placement_new_protocol,
    placement_in_syntax,
    pointer_methods,
    ptr_internals,
    raw,
    unique,
    unsize,
)]

use std::heap::{Alloc, AllocErr, Heap, Layout};
use std::marker::PhantomData;
use std::ptr::Unique;

mod boxed;
pub mod composable;
pub mod freelist;
pub mod scoped;

pub use boxed::{AllocBox, Place, make_place};
pub use composable::*;
pub use freelist::FreeList;
pub use scoped::Scoped;

#[inline]
fn allocate<T,A: Alloc + ?Sized>(allocator: &mut A, val: T) -> Result<AllocBox<T, A>, AllocErr> {
    make_place::<A, T>(allocator).map(|place| in place {val})
}

/// An allocator that knows which blocks have been issued by it.
pub trait BlockOwner: Alloc {
    /// Whether this allocator owns this allocated value. 
    fn owns<'a, T, A: Alloc>(&self, val: &AllocBox<'a, T, A>) -> bool {
        self.owns_block(val.as_ptr() as *mut u8, val.layout())
    }

    /// Whether this allocator owns the block passed to it.
    fn owns_block(&self, ptr: *mut u8, layout: Layout) -> bool;

    /// Joins this allocator with a fallback allocator.
    // TODO: Maybe not the right place for this?
    // Right now I've been more focused on shaking out the
    // specifics of allocation than crafting a fluent API.
    fn with_fallback<O: BlockOwner>(self, other: O) -> Fallback<Self, O>
        where Self: Sized
    {
        Fallback::new(self, other)
    }
}

/// A block of memory created by an allocator.
pub struct Block<'a> {
    ptr: Unique<u8>,
    layout: Layout,
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> Block<'a> {
    /// Create a new block from the supplied parts.
    /// The pointer cannot be null.
    ///
    /// # Panics
    /// Panics if the pointer passed is null.
    pub fn new(ptr: *mut u8, layout: Layout) -> Self {
        assert!(!ptr.is_null());
        Block {
            ptr: Unique::new(ptr).unwrap(),
            layout: layout,
            _marker: PhantomData,
        }
    }

    /// Creates an empty block.
    pub fn empty() -> Self {
        Block {
            ptr: Unique::empty(),
            layout: Layout::from_size_align(0,0).unwrap(),
            _marker: PhantomData,
        }
    }

    /// Get the pointer from this block.
    pub fn ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
    /// Get the size of this block.
    pub fn size(&self) -> usize {
        self.layout.size()
    }
    pub fn layout(&self) -> Layout {
        self.layout.clone()
    }
    /// Get the align of this block.
    pub fn align(&self) -> usize {
        self.layout.align()
    }
    /// Whether this block is empty.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }
}

/// Errors that can occur while creating an allocator
/// or allocating from it.
pub struct Error{}

impl Error {
    pub fn unsupported_alignment() -> AllocErr { AllocErr::invalid_input("unsupported alignment") }
    pub fn out_of_memory(request: Layout) -> AllocErr { AllocErr::Exhausted {request}}
}

// aligns a pointer forward to the next value aligned with `align`.
#[inline]
fn align_forward(ptr: *mut u8, align: usize) -> *mut u8 {
    ((ptr as usize + align - 1) & !(align - 1)) as *mut u8
}

#[cfg(test)]
mod tests {

    use std::any::Any;

    use super::*;

    #[test]
    fn heap_lifetime() {
        let my_int;
        {
            my_int = Unique::from(Heap::default().alloc_one::<i32>().unwrap());
        }

        assert_eq!(*my_int.as_ref(), 0);
    }
    #[test]
    fn heap_in_place() {
        let big = in make_place(&mut Heap::default()).unwrap() { [0u8; 8_000_000] };
        assert_eq!(big.len(), 8_000_000);
    }

    #[test]
    fn unsizing() {
        #[derive(Debug)]
        struct Bomb;
        impl Drop for Bomb {
            fn drop(&mut self) {
                println!("Boom")
            }
        }

        let my_foo: AllocBox<Any, _> = boxed::allocate::<Bomb, _>(&mut Heap::default());
        let _: AllocBox<Bomb, _> = my_foo.downcast().ok().unwrap();
    }

    #[test]
    fn take_out() {
        let _: [u8; 1024] = allocate(&mut Heap::default(), [0; 1024]).ok().unwrap().take();
    }

    #[test]
    fn boxed_allocator() {
        #[derive(Debug)]
        struct Increment<'a>(&'a mut i32);
        impl<'a> Drop for Increment<'a> {
            fn drop(&mut self) {
                *self.0 += 1;
            }
        }

        let mut i = 0;
        let mut alloc: Box<Alloc> = Box::new(Heap::default());
        {
            let _ = allocate(&mut *alloc, Increment(&mut i)).unwrap();
        }
        assert_eq!(i, 1);
    }
}
