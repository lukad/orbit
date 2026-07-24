use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

const MAX_SHORT_STRING_LEN: usize = 40;
type ShortStringInterner = RefCell<HashMap<Box<[u8]>, Weak<[u8]>>>;

thread_local! {
    static SHORT_STRINGS: ShortStringInterner = RefCell::new(HashMap::new());
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LuaString(Rc<[u8]>);

impl LuaString {
    pub fn new(bytes: impl AsRef<[u8]>) -> Self {
        let bytes = bytes.as_ref();
        Self(intern_short_string(bytes).unwrap_or_else(|| Rc::from(bytes)))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn identity(&self) -> usize {
        self.0.as_ptr() as usize
    }
}

impl Default for LuaString {
    fn default() -> Self {
        Self::new([])
    }
}

impl Drop for LuaString {
    fn drop(&mut self) {
        if self.len() > MAX_SHORT_STRING_LEN || Rc::strong_count(&self.0) != 1 {
            return;
        }

        let _ = SHORT_STRINGS.try_with(|strings| {
            let mut strings = strings.borrow_mut();
            let is_interned_string = strings
                .get(self.as_bytes())
                .is_some_and(|string| std::ptr::eq(string.as_ptr(), Rc::as_ptr(&self.0)));

            if is_interned_string {
                strings.remove(self.as_bytes());
            }
        });
    }
}

impl AsRef<[u8]> for LuaString {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<&[u8]> for LuaString {
    fn from(bytes: &[u8]) -> Self {
        Self::new(bytes)
    }
}

impl From<&str> for LuaString {
    fn from(value: &str) -> Self {
        Self::new(value.as_bytes())
    }
}

impl From<Vec<u8>> for LuaString {
    fn from(bytes: Vec<u8>) -> Self {
        if bytes.len() <= MAX_SHORT_STRING_LEN {
            Self::new(bytes)
        } else {
            Self(Rc::from(bytes.into_boxed_slice()))
        }
    }
}

impl From<Box<[u8]>> for LuaString {
    fn from(bytes: Box<[u8]>) -> Self {
        if bytes.len() <= MAX_SHORT_STRING_LEN {
            Self::new(bytes)
        } else {
            Self(Rc::from(bytes))
        }
    }
}

impl From<Rc<[u8]>> for LuaString {
    fn from(bytes: Rc<[u8]>) -> Self {
        if bytes.len() <= MAX_SHORT_STRING_LEN {
            Self::new(bytes)
        } else {
            Self(bytes)
        }
    }
}

impl std::fmt::Debug for LuaString {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("LuaString")
            .field(&self.as_bytes())
            .finish()
    }
}

fn intern_short_string(bytes: &[u8]) -> Option<Rc<[u8]>> {
    if bytes.len() > MAX_SHORT_STRING_LEN {
        return None;
    }

    SHORT_STRINGS.with(|strings| {
        let existing = strings.borrow().get(bytes).and_then(Weak::upgrade);
        if existing.is_some() {
            return existing;
        }

        let string = Rc::<[u8]>::from(bytes);
        strings
            .borrow_mut()
            .insert(bytes.into(), Rc::downgrade(&string));
        Some(string)
    })
}

#[cfg(test)]
mod tests {
    use super::{LuaString, MAX_SHORT_STRING_LEN};

    #[test]
    fn equal_short_strings_share_identity() {
        let first = LuaString::from(vec![b'a'; MAX_SHORT_STRING_LEN]);
        let second = LuaString::from(vec![b'a'; MAX_SHORT_STRING_LEN]);

        assert_eq!(first.identity(), second.identity());
    }

    #[test]
    fn equal_long_strings_keep_distinct_identities() {
        let first = LuaString::from(vec![b'a'; MAX_SHORT_STRING_LEN + 1]);
        let second = LuaString::from(vec![b'a'; MAX_SHORT_STRING_LEN + 1]);

        assert_ne!(first.identity(), second.identity());
    }
}
