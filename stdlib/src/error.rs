use orbit_vm::{VmError, VmErrorKind};

pub(crate) fn missing_value(function: &'static str, argument: usize) -> VmError {
    failure(format!(
        "bad argument #{argument} to '{function}' (value expected)"
    ))
}

pub(crate) fn type_error(
    function: &'static str,
    argument: usize,
    expected: &'static str,
    actual: Option<&'static str>,
) -> VmError {
    let actual = actual.unwrap_or("no value");

    failure(format!(
        "bad argument #{argument} to '{function}' ({expected} expected, got {actual})"
    ))
}

pub(crate) fn index_out_of_range(function: &'static str, argument: usize) -> VmError {
    failure(format!(
        "bad argument #{argument} to '{function}' (index out of range)"
    ))
}

pub(crate) fn number_has_no_integer_representation(
    function: &'static str,
    argument: usize,
) -> VmError {
    failure(format!(
        "bad argument #{argument} to '{function}' (number has no integer representation)"
    ))
}

pub(crate) fn failure(message: impl Into<Box<str>>) -> VmError {
    VmErrorKind::NativeFunctionFailure {
        message: message.into(),
    }
    .into()
}

pub(crate) fn assertion_failed() -> VmError {
    VmErrorKind::NativeFunctionFailure {
        message: "assertion failed!".into(),
    }
    .into()
}
