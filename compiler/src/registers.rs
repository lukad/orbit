use orbit_common::Span;

use crate::{
    bytecode::Register,
    error::{CompileError, CompileErrorKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct VReg(u16);

impl VReg {
    pub(crate) fn get(self) -> u16 {
        self.0
    }

    pub(crate) fn to_bytecode(self, span: Span) -> Result<Register, CompileError> {
        u8::try_from(self.0)
            .map(Register)
            .map_err(|_| CompileError {
                span,
                kind: CompileErrorKind::TooManyRegisters {
                    required: u32::from(self.0) + 1,
                },
            })
    }

    pub(crate) fn offset(self, offset: u16) -> Self {
        Self(
            self.0
                .checked_add(offset)
                .expect("virtual register overflow"),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RegRange {
    pub(crate) base: VReg,
    pub(crate) len: u16,
}

impl RegRange {
    pub(crate) fn iter(self) -> impl DoubleEndedIterator<Item = VReg> + ExactSizeIterator {
        (0..self.len).map(move |index| self.base.offset(index))
    }
}

pub(crate) struct RegisterStack {
    /// First register after pinned locals/loop state
    floor: u16,
    /// First unused temporary register
    top: u16,
    /// Maximum exclusive register index ever used
    max: u16,
}

impl RegisterStack {
    pub(crate) fn new() -> Self {
        Self {
            floor: 0,
            top: 0,
            max: 0,
        }
    }

    pub(crate) fn floor(&self) -> VReg {
        VReg(self.floor)
    }

    pub(crate) fn top(&self) -> VReg {
        VReg(self.top)
    }

    pub(crate) fn max_registers(&self) -> u16 {
        self.max
    }

    pub(crate) fn reserve_pinned(
        &mut self,
        count: u16,
        span: Span,
    ) -> Result<RegRange, CompileError> {
        assert_eq!(
            self.top, self.floor,
            "cannot reserve pinned registers while temporaries are live"
        );

        let base = self.floor;
        let end = self.checked_end(base, count, span)?;

        self.floor = end;
        self.top = end;
        self.max = self.max.max(end);

        Ok(RegRange {
            base: VReg(base),
            len: count,
        })
    }

    pub(crate) fn release_pinned_to(&mut self, base: VReg) {
        assert_eq!(
            self.top, self.floor,
            "cannot release pinned registers while temporaries are live"
        );
        assert!(
            base.get() <= self.floor,
            "cannot release pinned registers to a higher register"
        );

        self.floor = base.get();
        self.top = base.get();
    }

    pub(crate) fn promote_temporaries_to_pinned(&mut self, range: RegRange) {
        assert_eq!(
            range.base.get(),
            self.floor,
            "promoted temporary range must begin at the pinned floor"
        );
        assert_eq!(
            range.base.get() + range.len,
            self.top,
            "promoted temporary range must end at the temporary top"
        );

        self.floor = self.top;
    }

    pub(crate) fn temporary_mark(&self) -> VReg {
        VReg(self.top)
    }

    pub(crate) fn reserve_temporaries(
        &mut self,
        count: u16,
        span: Span,
    ) -> Result<RegRange, CompileError> {
        let base = self.top;
        let end = self.checked_end(base, count, span)?;

        self.top = end;
        self.max = self.max.max(end);

        Ok(RegRange {
            base: VReg(base),
            len: count,
        })
    }

    pub(crate) fn reserve_temporary_array<const N: usize>(
        &mut self,
        span: Span,
    ) -> Result<[VReg; N], CompileError> {
        let count = u16::try_from(N).map_err(|_| CompileError {
            span,
            kind: CompileErrorKind::TooManyRegisters {
                required: u32::try_from(N).unwrap_or(u32::MAX),
            },
        })?;
        let range = self.reserve_temporaries(count, span)?;

        Ok(std::array::from_fn(|index| range.base.offset(index as u16)))
    }

    pub(crate) fn release_temporaries_to(&mut self, mark: VReg) {
        assert!(
            mark.get() >= self.floor,
            "cannot release temporaries below pinned registers"
        );
        assert!(
            mark.get() <= self.top,
            "temporary mark is above the current top"
        );

        self.top = mark.get();
    }

    fn checked_end(&self, start: u16, count: u16, span: Span) -> Result<u16, CompileError> {
        let required = u32::from(start) + u32::from(count);

        if required > 256 {
            return Err(CompileError {
                span,
                kind: CompileErrorKind::TooManyRegisters { required },
            });
        }

        Ok(required as u16)
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::{SourceId, Span};

    use super::*;

    fn span() -> Span {
        Span::new(SourceId::new(0), 0, 0)
    }

    #[test]
    fn pinned_and_temporary_registers_are_stacked() {
        let mut registers = RegisterStack::new();

        let locals = registers.reserve_pinned(2, span()).unwrap();
        assert_eq!(locals.base.get(), 0);
        assert_eq!(locals.len, 2);

        let temps = registers.reserve_temporaries(3, span()).unwrap();
        assert_eq!(temps.base.get(), 2);
        assert_eq!(registers.max_registers(), 5);

        registers.release_temporaries_to(registers.floor());
        registers.release_pinned_to(VReg(0));

        assert_eq!(registers.floor().get(), 0);
        assert_eq!(registers.top().get(), 0);
        assert_eq!(registers.max_registers(), 5);
    }

    #[test]
    fn all_256_registers_are_addressable() {
        let mut registers = RegisterStack::new();
        let range = registers.reserve_pinned(256, span()).unwrap();

        assert_eq!(
            range.iter().last().unwrap().to_bytecode(span()).unwrap(),
            Register(255)
        );
        assert_eq!(registers.max_registers(), 256);
    }

    #[test]
    fn rejects_a_257th_register() {
        let mut registers = RegisterStack::new();
        registers.reserve_pinned(256, span()).unwrap();

        let error = registers.reserve_temporaries(1, span()).unwrap_err();

        assert!(matches!(
            error.kind,
            CompileErrorKind::TooManyRegisters { required: 257 }
        ));
    }

    #[test]
    fn promotes_an_exact_temporary_result_window_to_pinned() {
        let mut registers = RegisterStack::new();
        let results = registers.reserve_temporaries(3, span()).unwrap();

        registers.promote_temporaries_to_pinned(results);

        assert_eq!(registers.floor(), VReg(3));
        assert_eq!(registers.top(), VReg(3));

        let temporary = registers.reserve_temporaries(1, span()).unwrap();
        assert_eq!(temporary.base, VReg(3));
    }
}
