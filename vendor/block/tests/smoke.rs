extern crate block;

use block::ConcreteBlock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn stack_blocks_copy_retain_and_release_captured_values() {
    let drops = Arc::new(AtomicUsize::new(0));
    let probe = DropProbe(drops.clone());
    let captured = String::from("runtime block");
    let expected = captured.len();
    let stack = ConcreteBlock::new(move |extra: usize| {
        let _ = &probe;
        captured.len() + extra
    });
    assert_eq!(unsafe { stack.call((2,)) }, expected + 2);

    let copied = stack.copy();
    assert_eq!(unsafe { copied.call((3,)) }, expected + 3);
    let retained = copied.clone();
    drop(copied);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(unsafe { retained.call((4,)) }, expected + 4);
    drop(retained);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
