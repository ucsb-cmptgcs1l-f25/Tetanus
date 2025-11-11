use crate::codegen::aot;
use rustc_middle::ty::{Instance, InstanceKind};
use rustc_middle::ty::GenericArgsRef;
use rustc_span::def_id;

#[test]
pub fn test_empty_fn() {
    eprintln!("testing empty fn");
    assert_true!(
        aot::codegen_function(
            "test",
            Instance{
                def: InstanceKind::Item(DefId(0, 0, 0)),
                args: {},
            }
        ).contains(".test\n"),
    );
}
