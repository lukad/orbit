use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use orbit_vm::{LocalValue, NativeAction, NativeContext, State, Table, Value, VmResult};

use crate::{
    argument::{check_integer, required_integer},
    error,
};

pub const FUNCTION: &str = "random";
pub const SEED_FUNCTION: &str = "randomseed";

const STATE_WORDS: usize = 4;
const STATE_CAPTURE: usize = 0;

static SEED_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn create_state(state: &mut State) -> VmResult<Table> {
    let table = state.create_table(STATE_WORDS, 0)?;

    let (seed1, seed2) = default_seeds();
    let words = seeded_state(seed1, seed2);

    for (index, word) in words.into_iter().enumerate() {
        state.raw_set(
            &table,
            &Value::Integer((index + 1) as i64),
            &Value::Integer(word as i64),
        )?;
    }

    Ok(table)
}

fn default_seeds() -> (i64, i64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let counter = SEED_COUNTER.fetch_add(1, Ordering::Relaxed);

    let seed1 = now.as_secs();
    let seed2 = (u64::from(now.subsec_nanos()) << 32) ^ u64::from(std::process::id()) ^ counter;

    (seed1 as i64, seed2 as i64)
}

fn seeded_state(seed1: i64, seed2: i64) -> [u64; STATE_WORDS] {
    let mut state = [seed1 as u64, 0xff, seed2 as u64, 0];

    for _ in 0..16 {
        next_random(&mut state);
    }

    state
}

fn next_random(state: &mut [u64; STATE_WORDS]) -> u64 {
    let state0 = state[0];
    let state1 = state[1];
    let state2 = state[2] ^ state0;
    let state3 = state[3] ^ state1;

    let result = state1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);

    state[0] = state0 ^ state3;
    state[1] = state1 ^ state2;
    state[2] = state2 ^ state1.wrapping_shl(17);
    state[3] = state3.rotate_left(45);

    result
}

fn load_state<'context>(
    context: &mut NativeContext<'context>,
    table: &LocalValue<'context>,
) -> VmResult<[u64; STATE_WORDS]> {
    let mut words = [0; STATE_WORDS];

    for (index, word) in words.iter_mut().enumerate() {
        let key = context.integer((index + 1) as i64);
        let value = context.raw_get(table, &key)?;

        *word = value.as_integer().expect("random state contains integers") as u64;
    }

    Ok(words)
}

fn store_state<'context>(
    context: &mut NativeContext<'context>,
    table: &LocalValue<'context>,
    words: [u64; STATE_WORDS],
) -> VmResult<()> {
    for (index, word) in words.into_iter().enumerate() {
        context.raw_set(
            table,
            context.integer((index + 1) as i64),
            context.integer(word as i64),
        )?;
    }

    Ok(())
}

pub fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let state_table = context
        .capture(STATE_CAPTURE)
        .expect("math.random captures its state");

    let mut state = load_state(context, &state_table)?;
    let random = next_random(&mut state);

    store_state(context, &state_table, state)?;

    match context.argument_count() {
        0 => {
            let result = random_float(random);
            Ok(context.return_values([context.float(result)]))
        }
        1 => {
            let upper = required_integer(context, FUNCTION, 0)?;

            if upper == 0 {
                return Ok(context.return_values([context.integer(random as i64)]));
            }

            bounded_random(context, &state_table, state, random, 1, upper)
        }

        2 => {
            let lower = required_integer(context, FUNCTION, 0)?;
            let upper = required_integer(context, FUNCTION, 1)?;

            bounded_random(context, &state_table, state, random, lower, upper)
        }

        _ => Err(error::failure("wrong number of arguments")),
    }
}

fn bounded_random<'context>(
    context: &mut NativeContext<'context>,
    state_table: &orbit_vm::LocalValue<'context>,
    mut state: [u64; 4],
    random: u64,
    lower: i64,
    upper: i64,
) -> VmResult<NativeAction> {
    if lower > upper {
        return Err(error::argument_error(FUNCTION, 1, "interval is empty"));
    }

    let range = (upper as u64).wrapping_sub(lower as u64);
    let offset = project(random, range, &mut state);

    store_state(context, state_table, state)?;

    let result = offset.wrapping_add(lower as u64) as i64;

    Ok(context.return_values([context.integer(result)]))
}

fn project(mut random: u64, maximum: u64, state: &mut [u64; 4]) -> u64 {
    if maximum & maximum.wrapping_add(1) == 0 {
        return random & maximum;
    }

    let mut limit = maximum;
    limit |= limit >> 1;
    limit |= limit >> 2;
    limit |= limit >> 4;
    limit |= limit >> 8;
    limit |= limit >> 16;
    limit |= limit >> 32;

    loop {
        random &= limit;

        if random <= maximum {
            return random;
        }

        random = next_random(state);
    }
}

fn random_float(random: u64) -> f64 {
    const PRECISION_BITS: u32 = f64::MANTISSA_DIGITS;
    const SHIFT: u32 = u64::BITS - PRECISION_BITS;
    const SCALE: f64 = 1.0 / ((1_u64 << PRECISION_BITS) as f64);

    ((random >> SHIFT) as f64) * SCALE
}

pub(super) fn seed_callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let state_table = context
        .capture(STATE_CAPTURE)
        .expect("math.randomseed captures its state");

    let (seed1, seed2) = if context.argument_count() == 0 {
        default_seeds()
    } else {
        let seed1 = required_integer(context, SEED_FUNCTION, 0)?;

        let seed2 = match context.argument(1) {
            None => 0,
            Some(value) if value.is_nil() => 0,
            Some(value) => check_integer(&value, SEED_FUNCTION, 2)?,
        };

        (seed1, seed2)
    };

    let state = seeded_state(seed1, seed2);
    store_state(context, &state_table, state)?;

    Ok(context.return_values([context.integer(seed1), context.integer(seed2)]))
}
