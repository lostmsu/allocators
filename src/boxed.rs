use std::any::Any;
use std::borrow::{Borrow, BorrowMut};
use std::heap::{Alloc, AllocErr, Layout};
use std::marker::{PhantomData, Unsize};
use std::mem;
use std::ops::{CoerceUnsized, Deref, DerefMut, InPlace, Placer};
use std::ops::Place as StdPlace;
use std::ptr::Unique;

use super::Block;

/// An item allocated by a custom allocator.
pub struct AllocBox<'a, T: 'a + ?Sized, A: 'a + ?Sized + Alloc> {
    item: Unique<T>,
    layout: Layout,
    allocator: &'a A,
}

impl<'a, T: ?Sized, A: ?Sized + Alloc> AllocBox<'a, T, A> {
    /// Consumes this allocated value, yielding the value it manages.
    pub fn take(self) -> T where T: Sized {
        let val = unsafe { ::std::ptr::read(self.item.as_ptr()) };
        unsafe { self.allocator.dealloc(self.as_ptr() as *mut u8, self.layout) };
        mem::forget(self);
        val
    }

    pub fn as_ptr(&self) -> *mut T { self.item.as_ptr() }
    pub fn layout(&self) -> Layout { self.layout }
}

impl<'a, T: ?Sized, A: ?Sized + Alloc> Deref for AllocBox<'a, T, A> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { self.item.as_ref() }
    }
}

impl<'a, T: ?Sized, A: ?Sized + Alloc> DerefMut for AllocBox<'a, T, A> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.item.as_mut() }
    }
}

// AllocBox can store trait objects!
impl<'a, T: ?Sized + Unsize<U>, U: ?Sized, A: ?Sized + Alloc> CoerceUnsized<AllocBox<'a, U, A>> for AllocBox<'a, T, A> {}

impl<'a, A: ?Sized + Alloc> AllocBox<'a, Any, A> {
    /// Attempts to downcast this `AllocBox` to a concrete type.
    pub fn downcast<T: Any>(self) -> Result<AllocBox<'a, T, A>, AllocBox<'a, Any, A>> {
        use std::raw::TraitObject;
        if self.is::<T>() {
            let obj: TraitObject = unsafe { mem::transmute::<*mut Any, TraitObject>(self.item.as_ptr()) };
            let new_allocated = AllocBox {
                item: unsafe { Unique::new(obj.data as *mut T).unwrap() },
                layout: self.layout,
                allocator: self.allocator,
            };
            mem::forget(self);
            Ok(new_allocated)
        } else {
            Err(self)
        }
    }
}

impl<'a, T: ?Sized, A: ?Sized + Alloc> Borrow<T> for AllocBox<'a, T, A> {
    fn borrow(&self) -> &T {
        &**self
    }
}

impl<'a, T: ?Sized, A: ?Sized + Alloc> BorrowMut<T> for AllocBox<'a, T, A> {
    fn borrow_mut(&mut self) -> &mut T {
        &mut **self
    }
}

impl<'a, T: ?Sized, A: ?Sized + Alloc> Drop for AllocBox<'a, T, A> {
    #[inline]
    fn drop(&mut self) {
        use std::intrinsics::drop_in_place;
        unsafe {
            drop_in_place(self.item.as_ptr());
            self.allocator.dealloc(self.item.as_ptr() as *mut u8, self.layout);
        }

    }
}


pub fn make_place<A: ?Sized + Alloc, T>(alloc: &mut A) -> Result<Place<T, A>, AllocErr> {
    let layout = Layout::from_size_align(mem::size_of::<T>(), mem::align_of::<T>()).unwrap();
    match unsafe { alloc.alloc(layout) } {
        Ok(ptr) => {
            Ok(Place {
                allocator: alloc,
                block: Block::new(ptr, layout),
                _marker: PhantomData,
            })
        }
        Err(e) => Err(e),
    }
}

/// A place for allocating into.
/// This is only used for in-place allocation,
/// e.g. `let val = in (alloc.make_place().unwrap()) { EXPR }`
pub struct Place<'a, T: 'a, A: 'a + ?Sized + Alloc> {
    allocator: &'a A,
    block: Block<'a>,
    _marker: PhantomData<T>,
}

impl<'a, T: 'a, A: 'a + ?Sized + Alloc> Placer<T> for Place<'a, T, A> {
    type Place = Self;
    fn make_place(self) -> Self {
        self
    }
}

impl<'a, T: 'a, A: 'a + ?Sized + Alloc> InPlace<T> for Place<'a, T, A> {
    type Owner = AllocBox<'a, T, A>;
    unsafe fn finalize(self) -> Self::Owner {
        let allocated = AllocBox {
            item: Unique::new(self.block.ptr() as *mut T).unwrap(),
            layout: self.block.layout(),
            allocator: self.allocator,
        };

        mem::forget(self);
        allocated
    }
}

impl<'a, T: 'a, A: 'a + ?Sized + Alloc> StdPlace<T> for Place<'a, T, A> {
    fn pointer(&mut self) -> *mut T {
        self.block.ptr() as *mut T
    }
}

impl<'a, T: 'a, A: 'a + ?Sized + Alloc> Drop for Place<'a, T, A> {
    #[inline]
    fn drop(&mut self) {
        // almost identical to AllocBox::Drop, but we don't drop
        // the value in place. If the finalize
        // method was never called, the expression
        // to create the value failed and the memory at the
        // pointer is still uninitialized, which we don't want to drop.
        unsafe {
            let old_block = mem::replace(&mut self.block, Block::empty());
            self.allocator.dealloc(old_block.ptr(), old_block.layout());
        }

    }
}