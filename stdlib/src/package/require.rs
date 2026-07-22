use orbit_vm::{
    LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmError, VmResult,
};

use super::check_string;
use crate::error;

const FUNCTION_NAME: &str = "require";

const LOADED_LOOKUP: NativeToken = NativeToken::new(1);
const SEARCHERS_LOOKUP: NativeToken = NativeToken::new(2);
const SEARCHER_CALL: NativeToken = NativeToken::new(3);
const LOADER_CALL: NativeToken = NativeToken::new(4);
const LOADED_RESULT_SET: NativeToken = NativeToken::new(5);
const FINAL_LOADED_LOOKUP: NativeToken = NativeToken::new(6);
const LOADED_TRUE_SET: NativeToken = NativeToken::new(7);

const FIRST_SEARCHER: u64 = 1;

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume {
            token: LOADED_LOOKUP,
        } => finish_loaded_lookup(context),
        NativeEvent::Resume {
            token: SEARCHERS_LOOKUP,
        } => finish_searchers_lookup(context),
        NativeEvent::Resume {
            token: SEARCHER_CALL,
        } => finish_searcher(context),
        NativeEvent::Resume { token: LOADER_CALL } => finish_loader(context),
        NativeEvent::Resume {
            token: LOADED_RESULT_SET,
        } => finish_loaded_result_set(context),
        NativeEvent::Resume {
            token: FINAL_LOADED_LOOKUP,
        } => finish_final_loaded_lookup(context),
        NativeEvent::Resume {
            token: LOADED_TRUE_SET,
        } => finish_loaded_true_set(context),
        NativeEvent::ResumeError { .. } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
        NativeEvent::Resume { token } => Err(invalid_token(token)),
    }
}

fn start(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = check_string(context, 0, FUNCTION_NAME)?;
    let loaded = loaded_table(context);

    Ok(context.get_with_continuation(loaded, name.clone(), [name], LOADED_LOOKUP))
}

fn finish_loaded_lookup(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = continuation_value(context, 0)?;
    let cached = context.resume_value(0).unwrap_or_else(|| context.nil());

    if cached.is_truthy() {
        return Ok(context.return_values([cached]));
    }

    let package = context
        .capture(0)
        .expect("require captures the package table");
    let searchers_key = context.string("searchers");

    Ok(context.get_with_continuation(package, searchers_key, [name], SEARCHERS_LOOKUP))
}

fn finish_searchers_lookup(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = continuation_value(context, 0)?;
    let searchers = context.resume_value(0).unwrap_or_else(|| context.nil());

    if searchers.type_name() != "table" {
        return Err(error::failure("'package.searchers' must be a table"));
    }

    call_searcher(context, name, searchers, FIRST_SEARCHER, Vec::new())
}

fn finish_searcher(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = continuation_value(context, 0)?;
    let searchers = continuation_value(context, 1)?;
    let errors = continuation_value(context, 2)?;
    let index = continuation_value(context, 3)?
        .as_integer()
        .and_then(|index| u64::try_from(index).ok())
        .ok_or_else(|| error::failure("invalid require searcher index continuation"))?;

    let candidate = context.resume_value(0).unwrap_or_else(|| context.nil());

    if candidate.type_name() == "function" {
        let loader_data = context.resume_value(1).unwrap_or_else(|| context.nil());

        return Ok(context.call_with_continuation(
            candidate,
            [name.clone(), loader_data.clone()],
            [name, loader_data],
            LOADER_CALL,
        ));
    }

    let mut accumulated = errors
        .as_string()
        .ok_or_else(|| error::failure("invalid require error continuation"))?
        .as_bytes()
        .to_vec();

    let message = match candidate.type_name() {
        "string" => Some(candidate),
        "number" => Some(context.default_tostring(&candidate, None)),
        _ => None,
    };

    if let Some(message) = message {
        accumulated.extend_from_slice(b"\n\t");
        accumulated.extend_from_slice(
            message
                .as_string()
                .expect("searcher error is converted to a string")
                .as_bytes(),
        );
    }

    let next = index
        .checked_add(1)
        .ok_or_else(|| error::failure("too many package searchers"))?;

    call_searcher(context, name, searchers, next, accumulated)
}

fn call_searcher<'context>(
    context: &mut NativeContext<'context>,
    name: LocalValue<'context>,
    searchers: LocalValue<'context>,
    index: u64,
    errors: Vec<u8>,
) -> VmResult<NativeAction> {
    let index_value =
        i64::try_from(index).map_err(|_| error::failure("too many package searchers"))?;
    let key = context.integer(index_value);
    let searcher = context.raw_get(&searchers, &key)?;

    if searcher.is_nil() {
        return Err(module_not_found(&name, &errors));
    }

    let errors = context.string(errors);
    let index = context.integer(index_value);

    Ok(context.call_with_continuation(
        searcher,
        [name.clone()],
        [name, searchers, errors, index],
        SEARCHER_CALL,
    ))
}

fn finish_loader(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = continuation_value(context, 0)?;
    let loader_data = continuation_value(context, 1)?;
    let result = context.resume_value(0).unwrap_or_else(|| context.nil());

    if result.is_nil() {
        return final_loaded_lookup(context, name, loader_data);
    }

    let loaded = loaded_table(context);

    Ok(context.set_with_continuation(
        loaded,
        name.clone(),
        result,
        [name, loader_data],
        LOADED_RESULT_SET,
    ))
}

fn finish_loaded_result_set(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = continuation_value(context, 0)?;
    let loader_data = continuation_value(context, 1)?;

    final_loaded_lookup(context, name, loader_data)
}

fn final_loaded_lookup<'context>(
    context: &NativeContext<'context>,
    name: LocalValue<'context>,
    loader_data: LocalValue<'context>,
) -> VmResult<NativeAction> {
    let loaded = loaded_table(context);

    Ok(context.get_with_continuation(
        loaded,
        name.clone(),
        [name, loader_data],
        FINAL_LOADED_LOOKUP,
    ))
}

fn finish_final_loaded_lookup(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = continuation_value(context, 0)?;
    let loader_data = continuation_value(context, 1)?;
    let module = context.resume_value(0).unwrap_or_else(|| context.nil());

    if !module.is_nil() {
        return Ok(context.return_values([module, loader_data]));
    }

    let loaded = loaded_table(context);
    let default = context.boolean(true);

    Ok(context.set_with_continuation(loaded, name, default, [loader_data], LOADED_TRUE_SET))
}

fn finish_loaded_true_set(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let loader_data = continuation_value(context, 0)?;

    Ok(context.return_values([context.boolean(true), loader_data]))
}

fn loaded_table<'context>(context: &NativeContext<'context>) -> LocalValue<'context> {
    context.capture(1).expect("require captures package.loaded")
}

fn continuation_value<'context>(
    context: &NativeContext<'context>,
    index: usize,
) -> VmResult<LocalValue<'context>> {
    context
        .continuation_value(index)
        .ok_or_else(|| error::failure("missing require continuation value"))
}

fn module_not_found(name: &LocalValue<'_>, search_errors: &[u8]) -> VmError {
    let name = name.as_string().expect("checked module name is a string");

    let mut message = b"module '".to_vec();
    message.extend_from_slice(name.as_bytes());
    message.extend_from_slice(b"' not found:");
    message.extend_from_slice(search_errors);

    error::failure(String::from_utf8_lossy(&message).into_owned())
}

fn invalid_token(token: NativeToken) -> VmError {
    error::failure(format!(
        "invalid continuation token {} in '{FUNCTION_NAME}'",
        token.get(),
    ))
}
