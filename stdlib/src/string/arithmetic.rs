use orbit_vm::{ArithmeticOp, NativeAction, NativeContext, NativeEvent, NativeToken, VmResult};

use crate::error;

const FALLBACK_CALL: NativeToken = NativeToken::new(1);

pub(crate) fn add(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    callback(context, ArithmeticOp::Add, b"__add")
}

pub(crate) fn subtract(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    callback(context, ArithmeticOp::Subtract, b"__sub")
}

pub(crate) fn multiply(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    callback(context, ArithmeticOp::Multiply, b"__mul")
}

pub(crate) fn divide(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    callback(context, ArithmeticOp::Divide, b"__div")
}

pub(crate) fn floor_divide(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    callback(context, ArithmeticOp::FloorDivide, b"__idiv")
}

pub(crate) fn modulo(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    callback(context, ArithmeticOp::Modulo, b"__mod")
}

pub(crate) fn power(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    callback(context, ArithmeticOp::Power, b"__pow")
}

fn callback(
    context: &mut NativeContext<'_>,
    operation: ArithmeticOp,
    metamethod_name: &'static [u8],
) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context, operation, metamethod_name),
        NativeEvent::Resume {
            token: FALLBACK_CALL,
        } => {
            let result = context.resume_value(0).unwrap_or_default();
            Ok(context.return_values([result]))
        }
        NativeEvent::ResumeError {
            token: FALLBACK_CALL,
        } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),

        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => {
            Err(error::failure(format!(
                "invalid continuation token {} in string arithmetic metamethod",
                token.get(),
            )))
        }
    }
}

fn start(
    context: &mut NativeContext<'_>,
    operation: ArithmeticOp,
    metamethod_name: &'static [u8],
) -> VmResult<NativeAction> {
    let left = context.argument(0).unwrap_or_default();
    let right = context.argument(1).unwrap_or_default();

    if let (Some(left_number), Some(right_number)) = (left.to_number(), right.to_number()) {
        let result = context.raw_arithmetic(operation, &left_number, &right_number)?;

        return Ok(context.return_values([result]));
    }

    if right.type_name() != "string"
        && let Some(metatable) = context.get_metatable(&right)?
    {
        let key = context.string(metamethod_name);
        let fallback = context.raw_get(&metatable, &key)?;

        if !fallback.is_nil() {
            return Ok(context.call(fallback, [left, right], FALLBACK_CALL));
        }
    }

    match context.raw_arithmetic(operation, &left, &right) {
        Err(error) => Err(error),
        Ok(_) => unreachable!("unconverted operands unexpectedly supported arithmetic"),
    }
}
