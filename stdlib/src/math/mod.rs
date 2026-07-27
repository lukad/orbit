mod cos;
mod extrema;
mod floor;
mod fmod;
mod max;
mod min;
mod sin;
mod tan;
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

    let fmod = state.create_native_function("math.fmod", fmod::callback, &[])?;
    set_field(state, &math, fmod::FUNCTION, &Value::Function(fmod))?;

    let floor = state.create_native_function("math.floor", floor::callback, &[])?;
    set_field(state, &math, floor::FUNCTION, &Value::Function(floor))?;

    let sin = state.create_native_function("math.sin", sin::callback, &[])?;
    set_field(state, &math, sin::FUNCTION, &Value::Function(sin))?;

    let cos = state.create_native_function("math.cos", cos::callback, &[])?;
    set_field(state, &math, cos::FUNCTION, &Value::Function(cos))?;

    let tan = state.create_native_function("math.tan", tan::callback, &[])?;
    set_field(state, &math, tan::FUNCTION, &Value::Function(tan))?;

    set_field(state, &math, b"mininteger", &Value::Integer(i64::MIN))?;
    set_field(state, &math, b"maxinteger", &Value::Integer(i64::MAX))?;

    state.set_global(b"math", &Value::Table(math))
}
