use crate::{
    error::{FaultResult, VmErrorKind},
    value::RawValue,
};

pub(crate) struct ForPreparation {
    index: RawValue,
    limit: RawValue,
    step: RawValue,
    visible: Option<RawValue>,
}

impl ForPreparation {
    pub(crate) fn into_parts(self) -> (RawValue, RawValue, RawValue, Option<RawValue>) {
        (self.index, self.limit, self.step, self.visible)
    }
}

pub(crate) struct ForAdvance {
    index: RawValue,
    visible: Option<RawValue>,
}

impl ForAdvance {
    pub(crate) fn into_parts(self) -> (RawValue, Option<RawValue>) {
        (self.index, self.visible)
    }
}

pub(crate) fn prepare_numeric_for(
    initial: &RawValue,
    limit: &RawValue,
    step: &RawValue,
) -> FaultResult<ForPreparation> {
    if let (RawValue::Integer(initial), RawValue::Integer(step)) = (initial, step) {
        if *step == 0 {
            return Err(VmErrorKind::ZeroForStep);
        }

        let Some(limit) = integer_for_limit(limit, *step)? else {
            return Ok(ForPreparation {
                index: initial_value(*initial),
                limit: limit.clone(),
                step: RawValue::Integer(*step),
                visible: None,
            });
        };

        let enters = if *step > 0 {
            *initial <= limit
        } else {
            *initial >= limit
        };

        return Ok(ForPreparation {
            index: initial_value(*initial),
            limit: RawValue::Integer(limit),
            step: RawValue::Integer(*step),
            visible: enters.then_some(RawValue::Integer(*initial)),
        });
    }

    let initial = initial.to_float().ok_or(VmErrorKind::InvalidForControl)?;

    let limit = limit.to_float().ok_or(VmErrorKind::InvalidForControl)?;

    let step = step.to_float().ok_or(VmErrorKind::InvalidForControl)?;

    if step == 0.0 {
        return Err(VmErrorKind::ZeroForStep);
    }

    let enters = if step > 0.0 {
        initial <= limit
    } else {
        initial >= limit
    };

    Ok(ForPreparation {
        index: RawValue::Float(initial),
        limit: RawValue::Float(limit),
        step: RawValue::Float(step),
        visible: enters.then_some(RawValue::Float(initial)),
    })
}

pub(crate) fn advance_numeric_for(
    index: &RawValue,
    limit: &RawValue,
    step: &RawValue,
) -> FaultResult<ForAdvance> {
    match (index, limit, step) {
        (RawValue::Integer(index), RawValue::Integer(limit), RawValue::Integer(step)) => {
            if *step == 0 {
                return Err(VmErrorKind::ZeroForStep);
            }

            let Some(next) = index.checked_add(*step) else {
                return Ok(ForAdvance {
                    index: RawValue::Integer(*index),
                    visible: None,
                });
            };

            let continues = if *step > 0 {
                next <= *limit
            } else {
                next >= *limit
            };

            Ok(ForAdvance {
                index: RawValue::Integer(next),
                visible: continues.then_some(RawValue::Integer(next)),
            })
        }
        _ => {
            let index = index.to_float().ok_or(VmErrorKind::InvalidForControl)?;

            let limit = limit.to_float().ok_or(VmErrorKind::InvalidForControl)?;

            let step = step.to_float().ok_or(VmErrorKind::InvalidForControl)?;

            if step == 0.0 {
                return Err(VmErrorKind::ZeroForStep);
            }

            let next = index + step;

            let continues = if step > 0.0 {
                next <= limit
            } else {
                next >= limit
            };

            Ok(ForAdvance {
                index: RawValue::Float(next),
                visible: continues.then_some(RawValue::Float(next)),
            })
        }
    }
}

fn integer_for_limit(limit: &RawValue, step: i64) -> FaultResult<Option<i64>> {
    match limit {
        RawValue::Integer(limit) => Ok(Some(*limit)),
        RawValue::Float(limit) => {
            if limit.is_nan() {
                return Ok(None);
            }

            let minimum = i64::MIN as f64;
            let exclusive_maximum = -(i64::MIN as f64);

            if step > 0 {
                if *limit < minimum {
                    Ok(None)
                } else if *limit >= exclusive_maximum {
                    Ok(Some(i64::MAX))
                } else {
                    Ok(Some(limit.floor() as i64))
                }
            } else if *limit >= exclusive_maximum {
                Ok(None)
            } else if *limit < minimum {
                Ok(Some(i64::MIN))
            } else {
                Ok(Some(limit.ceil() as i64))
            }
        }
        _ => Err(VmErrorKind::InvalidForControl),
    }
}

fn initial_value(value: i64) -> RawValue {
    RawValue::Integer(value)
}
