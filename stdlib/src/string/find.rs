use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{
    argument,
    string::{
        offsets::start_offset,
        pattern::{self, CaptureValue},
    },
};

pub(crate) const FUNCTION: &str = "find";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let subject = argument::required_string(context, FUNCTION, 0)?;
    let subject = subject
        .as_string()
        .expect("required string is a string")
        .as_bytes();

    let pat = argument::required_string(context, FUNCTION, 1)?;
    let pat = pat
        .as_string()
        .expect("required string is a string")
        .as_bytes();

    let raw_start = match context.argument(2) {
        None => 1,
        Some(value) if value.is_nil() => 1,
        Some(value) => argument::check_integer(&value, FUNCTION, 2)?,
    };

    if raw_start > 0 && raw_start as u64 > subject.len() as u64 + 1 {
        return Ok(context.return_values([context.nil()]));
    }

    let start = start_offset(raw_start, subject.len());

    let plain = context.argument(3).unwrap_or_default().is_truthy()
        || !pat.iter().any(|b| b"^$*+?.([%-".contains(b));

    if plain {
        match memchr::memmem::find(&subject[start..], pat) {
            Some(i) => {
                let from = context.integer((i + start + 1) as i64);
                let to = context.integer((i + start + pat.len()) as i64);
                Ok(context.return_values([from, to]))
            }
            None => Ok(context.return_values([context.nil()])),
        }
    } else {
        match pattern::find(subject, pat, start)? {
            Some(m) => {
                let mut values = Vec::with_capacity(2 + m.captures.len());

                values.push(context.integer((m.start + 1) as i64));
                values.push(context.integer(m.end as i64));

                for capture in m.captures {
                    values.push(match capture {
                        CaptureValue::Text { start, end } => context.string(&subject[start..end]),
                        CaptureValue::Position(p) => context.integer(p as i64),
                    });
                }

                Ok(context.return_values(values))
            }
            None => Ok(context.return_values([context.nil()])),
        }
    }
}
