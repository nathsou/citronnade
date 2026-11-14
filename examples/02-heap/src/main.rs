//! Exercises the application heap (through `alloc`) and the linear heap, and
//! shows that running out of memory can be handled.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;

use citronnade::{heap, log, time};

#[citronnade::main]
fn main() -> Result<(), String> {
    let start = time::micros();
    let baseline = heap::app_stats().used;

    // Many small allocations of different sizes, freed in a scrambled order.
    let mut boxes: Vec<Box<[u8]>> = (0..2000usize)
        .map(|i| alloc::vec![(i % 251) as u8; 1 + (i * 37) % 600].into_boxed_slice())
        .collect();
    let mut seed = 12345u32;
    while !boxes.is_empty() {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let i = (seed >> 8) as usize % boxes.len();
        let b = boxes.swap_remove(i);
        if b.iter().any(|&x| x != b[0]) {
            return Err(format!("corrupted block of {} bytes", b.len()));
        }
    }
    drop(boxes); // the emptied vector still holds its capacity

    // Collections.
    let mut words: BTreeMap<String, usize> = BTreeMap::new();
    let text = "the quick brown fox jumps over the lazy dog the end";
    for w in text.split(' ') {
        *words.entry(String::from(w)).or_default() += 1;
    }
    log!("word counts: {:?}", words);
    if words["the"] != 3 {
        return Err(String::from("wrong word count"));
    }
    drop(words);

    let after = heap::app_stats();
    log!(
        "application heap: {} KiB used of {} KiB (baseline {} B, now {} B), largest free block {} KiB",
        after.used / 1024,
        after.size / 1024,
        baseline,
        after.used,
        after.largest_free / 1024
    );
    if after.used != baseline {
        return Err(format!("leaked {} bytes", after.used as isize - baseline as isize));
    }

    // The linear heap: 1 MiB, 128-byte aligned, as the GPU likes.
    let layout = Layout::from_size_align(1 << 20, 128).unwrap();
    let block = heap::linear_alloc(layout).ok_or("linear heap exhausted")?;
    if block.as_ptr() as usize % 128 != 0 || !heap::is_linear(block.as_ptr() as usize) {
        return Err(String::from("bad linear block"));
    }
    unsafe { block.as_ptr().write_bytes(0x5A, layout.size()) };
    log!("linear block at {:#010x}; linear heap {} KiB used", block.as_ptr() as usize, heap::linear_stats().used / 1024);
    unsafe { heap::linear_free(block, layout) };

    // Running out of memory is an error we can handle, not a crash.
    let mut huge: Vec<u8> = Vec::new();
    match huge.try_reserve(512 << 20) {
        Ok(()) => return Err(String::from("reserved 512 MiB?")),
        Err(e) => log!("as expected, 512 MiB is too much: {e}"),
    }

    log!("heap checks passed in {} us", time::micros() - start);
    Ok(())
}
