//! A Free List allocator.

use std::cell::Cell;
use std::heap::{Alloc, AllocErr, Layout};
use std::mem;
use std::ptr;

use super::Error;

/// A `FreeList` allocator manages a list of free memory blocks of uniform size.
/// Whenever a block is requested, it returns the first free block.
pub struct FreeList<'a, A: 'a + Alloc> {
    alloc: &'a mut A,
    block_size: usize,
    free_list: Cell<*mut u8>,
}

//impl FreeList<'static, std::heap::Heap> {
//    /// Creates a new `FreeList` backed by the heap. `block_size` must be greater
//    /// than or equal to the size of a pointer.
//    pub fn new(block_size: usize, num_blocks: usize) -> Result<Self, AllocErr> {
//        FreeList::new_from(SYSTEM_HEAP, block_size, num_blocks)
//    }
//}
impl<'a, A: 'a + Alloc> FreeList<'a, A> {
    /// Creates a new `FreeList` backed by another allocator. `block_size` must be greater
    /// than or equal to the size of a pointer.
    pub fn new_from(alloc: &'a mut A,
                    block_size: usize,
                    num_blocks: usize)
                    -> Result<Self, AllocErr> {
        if block_size < mem::size_of::<*mut u8>() {
            return Err(AllocErr::invalid_input("Block size too small.".into()));
        }

        let mut free_list = ptr::null_mut();

        let block_layout = Layout::from_size_align(block_size, mem::align_of::<*mut u8>()).unwrap();
        // allocate each block with maximal alignment.
        for _ in 0..num_blocks {
            match unsafe { alloc.alloc(block_layout) } {
                Ok(ptr) => {
                    let next_block_ptr: *mut *mut u8 = ptr as *mut *mut u8;
                    unsafe { *next_block_ptr = free_list }
                    free_list = ptr;
                }
                Err(err) => {
                    // destructor cleans up after us.
                    drop(FreeList {
                        alloc: alloc,
                        block_size: block_size,
                        free_list: Cell::new(free_list),
                    });

                    return Err(err);
                }
            }
        }

        Ok(FreeList {
            alloc: alloc,
            block_size: block_size,
            free_list: Cell::new(free_list),
        })
    }
}

unsafe impl<'a, A: 'a + Alloc> Alloc for FreeList<'a, A> {
    unsafe fn alloc(&mut self, layout: Layout) -> Result<*mut u8, AllocErr> {
        if layout.size() == 0 {
            return Err(AllocErr::invalid_input("Can't allocate 0 bytes"));
        } else if layout.size() > self.block_size {
            return Err(AllocErr::invalid_input("Allocation must be within block size"));
        }

        if layout.align() > mem::align_of::<*mut u8>() {
            return Err(Error::unsupported_alignment());
        }

        let free_list = self.free_list.get();
        if !free_list.is_null() {
            let next_block = *(free_list as *mut *mut u8);
            self.free_list.set(next_block);

            Ok(free_list)
        } else {
            Err(Error::out_of_memory(layout))
        }
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        if layout.size() != 0 {
            let first = self.free_list.get();
            *(ptr as *mut *mut u8) = first;
            self.free_list.set(ptr);
        }
    }
}

impl<'a, A: 'a + Alloc> Drop for FreeList<'a, A> {
    fn drop(&mut self) {
        let mut free_list = self.free_list.get();
        let block_layout = Layout::from_size_align(self.block_size, mem::align_of::<*mut u8>()).unwrap();
        //free all the blocks in the list.
        while !free_list.is_null() {
            unsafe {
                let next = *(free_list as *mut *mut u8);
                self.alloc.dealloc(free_list,block_layout);
                free_list = next;
            }
        }
    }
}

unsafe impl<'a, A: 'a + Alloc + Sync> Send for FreeList<'a, A> {}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn it_works() {
        let alloc = FreeList::new(1024, 64).ok().unwrap();
        let mut blocks = Vec::new();
        for _ in 0..64 {
            blocks.push(alloc.allocate([0u8; 1024]).ok().unwrap());
        }
        assert!(alloc.allocate([0u8; 1024]).is_err());
        drop(blocks);
        assert!(alloc.allocate([0u8; 1024]).is_ok());
    }
}
