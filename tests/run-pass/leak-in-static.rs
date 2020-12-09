use std::{ptr, sync::atomic::{AtomicPtr, Ordering}};

static mut LEAKER: Option<Box<Vec<i32>>> = None;

fn main() {
    // Having memory "leaked" in globals is allowed.
    unsafe {
        LEAKER = Some(Box::new(vec![0; 42]));
    }

    // Make sure this is allowed even when `AtomicPtr` is used.
    {
        static LEAK: AtomicPtr<usize> = AtomicPtr::new(ptr::null_mut());
        LEAK.store(Box::into_raw(Box::new(0usize)), Ordering::SeqCst);
    }
}
