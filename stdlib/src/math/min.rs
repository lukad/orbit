use orbit_vm::{NativeAction, NativeContext, VmResult};

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    super::extrema::callback(context, super::extrema::Kind::Minimum)
}
