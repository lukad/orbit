mod basic;
mod error;
mod package;

use orbit_vm::{State, VmResult};

pub fn install(state: &mut State) -> VmResult<()> {
    basic::install(state)?;
    package::install(state)
}
