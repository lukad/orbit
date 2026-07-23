mod max;

use orbit_vm::{State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let math = state.create_table(0, 1)?;

    let unpack = state.create_native_function("math.max", max::callback, &[])?;
    set_field(state, &math, b"max", &Value::Function(unpack))?;

    state.set_global(b"math", &Value::Table(math))
}
