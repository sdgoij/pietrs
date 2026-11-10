#![no_main]

//! Cranelift lowering. `emit_object` runs the CLIF verifier, so a target that
//! reaches here exercises the verifier as well as the lowering.

use libfuzzer_sys::fuzz_target;

#[path = "grid.rs"]
mod grid;

fuzz_target!(|data: &[u8]| {
    let Some(codel_grid) = grid::grid(data, 5) else {
        return;
    };
    let blocks = canvas::Blocks::partition(codel_grid);
    for division in [runtime::Division::Floored, runtime::Division::Truncating] {
        let object =
            codegen::emit_object(&blocks, division).expect("the host object emits for any graph");
        assert!(!object.is_empty(), "the object carries the program");
    }
});
