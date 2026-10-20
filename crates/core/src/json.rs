//! A minimal JSON object writer, for responses that embed raw JSON verbatim
//! and are built in a single buffer.

/// Writes `{"key":value,...}`; values are already-serialized JSON.
pub(crate) struct Obj(String);

impl Obj {
    pub fn new() -> Obj {
        Obj::with_capacity(64)
    }

    pub fn with_capacity(capacity: usize) -> Obj {
        let mut buf = String::with_capacity(capacity);
        buf.push('{');
        Obj(buf)
    }

    pub fn raw(mut self, key: &str, json: &str) -> Obj {
        self.key(key);
        self.0.push_str(json);
        self
    }

    pub fn str(self, key: &str, value: &str) -> Obj {
        self.raw(key, &quote(value))
    }

    /// Starts a nested object under `key`; finish it with [`Obj::close`].
    pub fn open(mut self, key: &str) -> Obj {
        self.key(key);
        self.0.push('{');
        self
    }

    pub fn close(mut self) -> Obj {
        self.0.push('}');
        self
    }

    pub fn end(mut self) -> String {
        self.0.push('}');
        self.0
    }

    fn key(&mut self, key: &str) {
        // Raw values never end with `{`, so this is exactly "no member yet".
        if !self.0.ends_with('{') {
            self.0.push(',');
        }
        self.0.push_str(&quote(key));
        self.0.push(':');
    }
}

/// A JSON string literal.
pub(crate) fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_nested_objects() {
        let json = Obj::new()
            .str("a", "x\"")
            .open("b")
            .raw("c", "[1]")
            .open("d")
            .close()
            .close()
            .raw("e", "true")
            .end();
        assert_eq!(json, r#"{"a":"x\"","b":{"c":[1],"d":{}},"e":true}"#);
        assert_eq!(Obj::new().end(), "{}");
    }
}
