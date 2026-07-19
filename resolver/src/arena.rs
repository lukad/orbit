use std::{
    marker::PhantomData,
    ops::{Index, IndexMut},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Arena<I, T> {
    values: Vec<T>,
    marker: PhantomData<fn(I) -> I>,
}

impl<I, T> Arena<I, T>
where
    I: Copy + From<u32> + Into<u32>,
{
    pub(crate) fn new() -> Self {
        Self {
            values: Vec::new(),
            marker: PhantomData,
        }
    }

    pub(crate) fn push(&mut self, value: T) -> I {
        let id =
            I::from(u32::try_from(self.values.len()).expect("HIR arena exceeded u32::MAX entries"));
        self.values.push(value);
        id
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.values.iter().enumerate().map(|(index, value)| {
            (
                I::from(u32::try_from(index).expect("HIR arena exceeded u32::MAX entries")),
                value,
            )
        })
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, id: I) -> Option<&T> {
        self.values.get(Into::<u32>::into(id) as usize)
    }
}

impl<I, T> Index<I> for Arena<I, T>
where
    I: Into<u32>,
{
    type Output = T;
    fn index(&self, id: I) -> &Self::Output {
        &self.values[Into::<u32>::into(id) as usize]
    }
}

impl<I, T> IndexMut<I> for Arena<I, T>
where
    I: Into<u32>,
{
    fn index_mut(&mut self, id: I) -> &mut Self::Output {
        &mut self.values[Into::<u32>::into(id) as usize]
    }
}
