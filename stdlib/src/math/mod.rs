mod extrema;
mod max;
mod min;
mod tointeger;

use orbit_vm::{State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let math = state.create_table(0, 2)?;

    let max = state.create_native_function("math.max", max::callback, &[])?;
    set_field(state, &math, b"max", &Value::Function(max))?;

    let min = state.create_native_function("math.min", min::callback, &[])?;
    set_field(state, &math, b"min", &Value::Function(min))?;

    let tointeger = state.create_native_function("math.tointeger", tointeger::callback, &[])?;
    set_field(state, &math, b"tointeger", &Value::Function(tointeger))?;

    set_field(state, &math, b"mininteger", &Value::Integer(i64::MIN))?;
    set_field(state, &math, b"maxinteger", &Value::Integer(i64::MAX))?;

    state.set_global(b"math", &Value::Table(math))
}
