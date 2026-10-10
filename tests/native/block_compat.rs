// SPDX-License-Identifier: MPL-2.0

use block::ConcreteBlock;
use std::cell::Cell;
use std::rc::Rc;

struct Capture(Rc<Cell<usize>>);

impl Drop for Capture {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn copied_blocks_preserve_arguments_captures_and_reference_counts() {
    let drops = Rc::new(Cell::new(0));
    let capture = Capture(drops.clone());
    let stack = ConcreteBlock::new(move |a: i32, b: i32| {
        assert_eq!(capture.0.get(), 0);
        a + b
    });
    assert_eq!(unsafe { stack.call((5, 8)) }, 13);
    let heap = stack.copy();
    let clone = heap.clone();
    assert_eq!(unsafe { heap.call((8, 13)) }, 21);
    drop(heap);
    assert_eq!(drops.get(), 0);
    assert_eq!(unsafe { clone.call((13, 21)) }, 34);
    drop(clone);
    assert_eq!(drops.get(), 1);
}

#[test]
fn copied_block_accepts_zero_arguments() {
    let block = ConcreteBlock::new(|| 42_u64).copy();
    assert_eq!(unsafe { block.call(()) }, 42);
}
