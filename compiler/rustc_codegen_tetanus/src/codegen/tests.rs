use aot;
use rustc_middle::ty::{Instance, InstanceKind};
use rustc_middle::ty::GenericArgsRef;
use rustc_span::def_id;

pub fn test_empty_fn() {
    assert_eq!(
        aot::codegen_function(
            "test",
            Instance{
                def: InstanceKind::Item(DefId(0, 0, 0)),
                args: {},
            }
        ),
        ".test\n",
    )
}