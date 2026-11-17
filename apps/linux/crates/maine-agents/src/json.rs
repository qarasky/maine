//! Untyped JSON helpers. Tool arguments and results are `serde_json::Value`s;
//! this module adds the accessors and the compact, key-sorted text form the
//! macOS client uses for MCP `text` content.

use serde_json::Value;

/// Accessors that mirror the Swift `JSONValue` helpers.
pub trait ValueExt {
    /// A string member, if present and a string.
    fn string(&self, key: &str) -> Option<&str>;
    /// An integral number member (a whole number below 1e15), if present.
    fn int(&self, key: &str) -> Option<i64>;
    /// An array member, if present and an array.
    fn array(&self, key: &str) -> Option<&Vec<Value>>;
}

impl ValueExt for Value {
    fn string(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    fn int(&self, key: &str) -> Option<i64> {
        as_int(self.get(key)?)
    }

    fn array(&self, key: &str) -> Option<&Vec<Value>> {
        self.get(key)?.as_array()
    }
}

/// A whole number below 1e15, from any JSON number spelling (`5` or `5.0`).
pub fn as_int(value: &Value) -> Option<i64> {
    let Value::Number(number) = value else {
        return None;
    };
    if let Some(i) = number.as_i64() {
        return (i.unsigned_abs() < 1_000_000_000_000_000).then_some(i);
    }
    let f = number.as_f64()?;
    (f.fract() == 0.0 && f.abs() < 1e15).then_some(f as i64)
}

/// Compact JSON with object keys sorted, whole numbers without a fraction.
pub fn json_text(value: &Value) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match as_int(value) {
            Some(i) if n.is_f64() => out.push_str(&i.to_string()),
            _ => out.push_str(&n.to_string()),
        },
        Value::String(s) => {
            out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()))
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into()));
                out.push(':');
                write(&map[key.as_str()], out);
            }
            out.push('}');
        }
    }
}

/// The spelling of a UUID on the wire and in files: uppercase, as Swift writes it.
pub fn uuid_text(id: &uuid::Uuid) -> String {
    id.hyphenated().to_string().to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_is_compact_and_sorted() {
        let value = json!({"b": [1, 2.0, "x/y"], "a": null, "c": {"z": true, "y": 1.5}});
        assert_eq!(
            json_text(&value),
            r#"{"a":null,"b":[1,2,"x/y"],"c":{"y":1.5,"z":true}}"#
        );
    }

    #[test]
    fn accessors_follow_the_swift_rules() {
        let value = json!({"s": "x", "i": 5, "f": 5.0, "frac": 5.5, "big": 1e16, "a": [1]});
        assert_eq!(value.string("s"), Some("x"));
        assert_eq!(value.string("i"), None);
        assert_eq!(value.int("i"), Some(5));
        assert_eq!(value.int("f"), Some(5));
        assert_eq!(value.int("frac"), None);
        assert_eq!(value.int("big"), None);
        assert_eq!(value.int("s"), None);
        assert!(value.array("a").is_some());
        assert!(value.array("s").is_none());
    }
}
