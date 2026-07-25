use orbit_vm::{GcMode, NativeAction, NativeContext, VmResult};

use crate::error;

pub(crate) const FUNCTION: &str = "collectgarbage";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let argument = context.argument(0);

    let option = match argument.as_ref() {
        None => b"collect",
        Some(value) if value.is_nil() => b"collect",
        Some(value) => value
            .as_string()
            .ok_or_else(|| error::type_error(FUNCTION, 1, "string", Some(value.type_name())))?
            .as_bytes(),
    };

    match option {
        b"collect" => {
            context.collect_garbage()?;
            Ok(context.return_values([context.integer(0)]))
        }
        b"count" => Ok(context.return_values([context.float(context.gc_memory_kbytes())])),
        b"stop" => {
            context.set_gc_running(false);
            Ok(context.return_values([context.integer(0)]))
        }
        b"restart" => {
            context.set_gc_running(true);
            Ok(context.return_values([context.integer(0)]))
        }
        b"step" => {
            context.collect_garbage()?;
            Ok(context.return_values([context.boolean(true)]))
        }
        b"isrunning" => Ok(context.return_values([context.boolean(context.gc_running())])),
        b"incremental" => {
            let prev = set_gc_mode(context, GcMode::Incremental);
            Ok(context.return_values([context.string(prev)]))
        }
        b"generational" => {
            let prev = set_gc_mode(context, GcMode::Generational);
            Ok(context.return_values([context.string(prev)]))
        }
        option => {
            let option = std::str::from_utf8(option).unwrap_or("<unknown>");
            Err(error::argument_error(
                FUNCTION,
                1,
                format!("invalid option '{option}'"),
            ))
        }
    }
}

fn set_gc_mode(context: &mut NativeContext<'_>, mode: GcMode) -> &'static str {
    match context.set_gc_mode(mode) {
        GcMode::Incremental => "incremental",
        GcMode::Generational => "generational",
    }
}
