use std::any::Any;
use std::borrow::{Borrow, BorrowMut};
use std::cell::RefCell;
use std::heap::{Alloc, AllocErr, Layout};
use std::marker::{PhantomData, Unsize};
use std::mem;
use std::ops::{CoerceUnsized, Deref, DerefMut, InPlace, Placer};
use std::ops::Place as StdPlace;
use std::ptr::Unique;
use std::rc::Rc;

use super::Block;

/// An item allocated by a custom allocator.
pub struct AllocBox<T: ?Sized, A: ?Sized + Alloc> {
    item: Option<Unique<T>>,
    layout: Layout,
    allocator: Rc<RefCell<A>>,
}

impl<T: ?Sized, A: ?Sized + Alloc> AllocBox<T, A> {
    /// Consumes this allocated value, yielding the value it manages.
    pub fn take(self) -> T where T: Sized {
        let item_ptr = self.item.take().unwrap();
        let val = unsafe { ::std::ptr::read(item_ptr.as_ptr()) };
        let ptr = self.as_ptr() as *mut u8;
        unsafe { self.allocator.borrow_mut().dealloc(ptr, self.layout.clone()) };
        val
    }

    pub fn as_ptr(&self) -> *mut T { self.item.unwrap().as_ptr() }
    pub fn layout(&self) -> Layout { self.layout.clone() }
}

impl<T: ?Sized, A: ?Sized + Alloc> Deref for AllocBox<T, A> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { self.item.as_ref() }
    }
}

impl<T: ?Sized, A: ?Sized + Alloc> DerefMut for AllocBox<T, A> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.item.as_mut() }
    }
}

// AllocBox can store trait objects!
// impl<T: ?Sized + Unsize<U>, U: ?Sized, A: ?Sized + Alloc> CoerceUnsized<AllocBox<U, A>> for AllocBox<T, A> {}

impl<A: ?Sized + Alloc> AllocBox<Any, A> {
    /// Attempts to downcast this `AllocBox` to a concrete type.
    pub fn downcast<T: Any>(self) -> Result<AllocBox<T, A>, AllocBox<Any, A>> where A: Sized {
        use std::raw::TraitObject;
        if self.is::<T>() {
            let obj: TraitObject = unsafe { mem::transmute::<*mut Any, TraitObject>(self.item.as_ptr()) };
            let new_allocated = AllocBox {
                item: Unique::new(obj.data as *mut T).unwrap(),
                layout: self.layout.clone(),
                allocator: unsafe { mem::transmute::<&mut A, &mut A>(self.allocator) },
            };
            mem::forget(self);
            Ok(new_allocated)
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized, A: ?Sized + Alloc> Borrow<T> for AllocBox<T, A> {
    fn borrow(&self) -> &T {
        &**self
    }
}

impl<T: ?Sized, A: ?Sized + Alloc> BorrowMut<T> for AllocBox<T, A> {
    fn borrow_mut(&mut self) -> &mut T {
        &mut **self
    }
}

impl<T: ?Sized, A: ?Sized + Alloc> Drop for AllocBox<T, A> {
    #[inline]
    fn drop(&mut self) {
        match self.item { }
        use std::intrinsics::drop_in_place;
        self.item.map(|item| {
            unsafe {
                drop_in_place(self.item.as_ptr());
                self.allocator.dealloc(self.item.as_ptr() as *mut u8, self.layout.clone());
            }
        });
    }
}


pub fn make_place<A: ?Sized + Alloc, T>(alloc: &mut A) -> Result<Place<T, A>, AllocErr> {
    let layout = Layout::from_size_align(mem::size_of::<T>(), mem::align_of::<T>()).unwrap();
    match unsafe { alloc.alloc(layout.clone()) } {
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
pub struct Place<T, A: ?Sized + Alloc> {
    allocator: Rc<RefCell<A>>,
    block: Block,
    _marker: PhantomData<T>,
}

impl<T, A: ?Sized + Alloc> Placer<T> for Place<T, A> {
    type Place = Self;
    fn make_place(self) -> Self {
        self
    }
}

impl<T, A: ?Sized + Alloc> InPlace<T> for Place<T, A> {
    type Owner = AllocBox<T, A>;
    unsafe fn finalize(self) -> Self::Owner {
        let allocated = AllocBox {
            item: Unique::new(self.block.ptr() as *mut T).unwrap(),
            layout: self.block.layout().clone(),
            allocator: mem::transmute::<&mut A, &mut A>(self.allocator),
        };

        mem::forget(self);
        allocated
    }
}

impl<T, A: ?Sized + Alloc> StdPlace<T> for Place<T, A> {
    fn pointer(&mut self) -> *mut T {
        self.block.ptr() as *mut T
    }
}

impl<T, A: ?Sized + Alloc> Drop for Place<T, A> {
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