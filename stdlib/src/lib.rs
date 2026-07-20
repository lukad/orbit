use std::{
    io::{self, Write},
    rc::Rc,
};

use orbit_vm::{Environment, Value, VmError, VmErrorKind, VmResult};

pub fn default_environment() -> VmResult<Environment> {
    let environment = Environment::new();
    install(&environment)?;
    Ok(environment)
}

pub fn install(environment: &Environment) -> VmResult<()> {
    environment.set(b"print", Value::native_function("print", print))
}

fn print(arguments: &[Value]) -> VmResult<Vec<Value>> {
    let stdout = io::stdout();
    let mut output = stdout.lock();

    for (index, arg) in arguments.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t").map_err(native_io_error)?;
        }

        write_value(&mut output, arg).map_err(native_io_error)?;
    }

    output.write_all(b"\n").map_err(native_io_error)?;
    output.flush().map_err(native_io_error)?;

    Ok(Vec::new())
}

fn write_value(output: &mut dyn Write, value: &Value) -> io::Result<()> {
    match value {
        Value::Nil => output.write_all(b"nil"),
        Value::Boolean(true) => output.write_all(b"true"),
        Value::Boolean(false) => output.write_all(b"false"),
        Value::Integer(value) => {
            write!(output, "{value}")
        }
        Value::Float(value) => output.write_all(format_float(*value).as_bytes()),
        Value::String(value) => output.write_all(value.as_ref()),
        Value::Table(value) => {
            write!(output, "table: {:p}", Rc::as_ptr(value))
        }
        Value::Closure(value) => {
            write!(output, "function: {:p}", Rc::as_ptr(value))
        }
        Value::NativeFunction(value) => {
            write!(output, "function: {:p}", Rc::as_ptr(value))
        }
    }
}

fn format_float(value: f64) -> String {
    let mut formatted = value.to_string();

    if value.is_finite()
        && !formatted.contains('.')
        && !formatted.contains('e')
        && !formatted.contains('E')
    {
        formatted.push_str(".0");
    }

    formatted
}

fn native_io_error(error: io::Error) -> VmError {
    VmErrorKind::NativeFunctionFailure {
        message: error.to_string().into_boxed_str(),
    }
    .into()
}
