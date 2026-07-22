use std::rc::Rc;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LuaString(Rc<[u8]>);

impl LuaString {
    pub fn new(bytes: impl AsRef<[u8]>) -> Self {
        Self(Rc::from(bytes.as_ref()))
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
}

impl Default for LuaString {
    fn default() -> Self {
        Self(Rc::from([]))
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
        Self(Rc::from(bytes.into_boxed_slice()))
    }
}

impl From<Box<[u8]>> for LuaString {
    fn from(bytes: Box<[u8]>) -> Self {
        Self(Rc::from(bytes))
    }
}

impl From<Rc<[u8]>> for LuaString {
    fn from(bytes: Rc<[u8]>) -> Self {
        Self(bytes)
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
