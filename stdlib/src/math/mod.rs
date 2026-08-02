mod abs;
mod acos;
mod asin;
mod atan;
mod ceil;
mod cos;
mod deg;
mod exp;
mod extrema;
mod floor;
mod fmod;
mod log;
mod max;
mod min;
mod modf;
mod rad;
mod sin;
mod sqrt;
mod tan;
mod tointeger;
mod r#type;
mod ult;

use std::f64;

use orbit_vm::{State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let math = state.create_table(0, 2)?;

    let abs = state.create_native_function("math.abs", abs::callback, &[])?;
    set_field(state, &math, abs::FUNCTION_NAME, &Value::Function(abs))?;

    let max = state.create_native_function("math.max", max::callback, &[])?;
    set_field(state, &math, b"max", &Value::Function(max))?;

    let min = state.create_native_function("math.min", min::callback, &[])?;
    set_field(state, &math, b"min", &Value::Function(min))?;

    let ult = state.create_native_function("math.ult", ult::callback, &[])?;
    set_field(state, &math, ult::FUNCTION_NAME, &Value::Function(ult))?;

    let tointeger = state.create_native_function("math.tointeger", tointeger::callback, &[])?;
    set_field(state, &math, b"tointeger", &Value::Function(tointeger))?;

    let fmod = state.create_native_function("math.fmod", fmod::callback, &[])?;
    set_field(state, &math, fmod::FUNCTION, &Value::Function(fmod))?;

    let floor = state.create_native_function("math.floor", floor::callback, &[])?;
    set_field(state, &math, floor::FUNCTION, &Value::Function(floor))?;

    let ceil = state.create_native_function("math.ceil", ceil::callback, &[])?;
    set_field(state, &math, ceil::FUNCTION, &Value::Function(ceil))?;

    let sin = state.create_native_function("math.sin", sin::callback, &[])?;
    set_field(state, &math, sin::FUNCTION, &Value::Function(sin))?;

    let cos = state.create_native_function("math.cos", cos::callback, &[])?;
    set_field(state, &math, cos::FUNCTION, &Value::Function(cos))?;

    let tan = state.create_native_function("math.tan", tan::callback, &[])?;
    set_field(state, &math, tan::FUNCTION, &Value::Function(tan))?;

    let asin = state.create_native_function("math.asin", asin::callback, &[])?;
    set_field(state, &math, asin::FUNCTION, &Value::Function(asin))?;

    let acos = state.create_native_function("math.acos", acos::callback, &[])?;
    set_field(state, &math, acos::FUNCTION, &Value::Function(acos))?;

    let atan = state.create_native_function("math.atan", atan::callback, &[])?;
    set_field(state, &math, atan::FUNCTION, &Value::Function(atan))?;

    let deg = state.create_native_function("math.deg", deg::callback, &[])?;
    set_field(state, &math, deg::FUNCTION, &Value::Function(deg))?;

    let rad = state.create_native_function("math.rad", rad::callback, &[])?;
    set_field(state, &math, rad::FUNCTION, &Value::Function(rad))?;

    let sqrt = state.create_native_function("math.sqrt", sqrt::callback, &[])?;
    set_field(state, &math, sqrt::FUNCTION, &Value::Function(sqrt))?;

    let exp = state.create_native_function("math.exp", exp::callback, &[])?;
    set_field(state, &math, exp::FUNCTION, &Value::Function(exp))?;

    let log = state.create_native_function("math.log", log::callback, &[])?;
    set_field(state, &math, log::FUNCTION, &Value::Function(log))?;

    let r#type = state.create_native_function("math.type", r#type::callback, &[])?;
    set_field(state, &math, r#type::FUNCTION, &Value::Function(r#type))?;

    let modf = state.create_native_function("math.modf", modf::callback, &[])?;
    set_field(state, &math, modf::FUNCTION, &Value::Function(modf))?;

    set_field(state, &math, b"mininteger", &Value::Integer(i64::MIN))?;
    set_field(state, &math, b"maxinteger", &Value::Integer(i64::MAX))?;
    set_field(state, &math, b"huge", &Value::Float(f64::INFINITY))?;
    set_field(state, &math, b"pi", &Value::Float(std::f64::consts::PI))?;

    state.set_global(b"math", &Value::Table(math))
}
