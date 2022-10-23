use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

/// An iterator over pairs of matching '{' and '}'
struct IterJson<'a> {
    s: &'a [u8],
    start_pos: Vec<usize>,
    current_pos: usize,
}

impl<'a> IterJson<'a> {
    fn new(s: &'a [u8]) -> Self {
        let start_pos = Vec::with_capacity(8);
        IterJson { s, start_pos, current_pos: 0 }
    }
}

impl<'a> Iterator for IterJson<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(c) = self.s.get(self.current_pos) {
            match c {
                b'{' => { self.start_pos.push(self.current_pos) }
                b'}' => {
                    if let Some(start) = self.start_pos.pop() {
                        self.current_pos += 1;
                        return Some(&self.s[start..self.current_pos]);
                    }
                }
                _ => {}
            }
            self.current_pos += 1;
        }
        None
    }
}

/// Return an iterator over all JSON values that can be deserialized in the given byte buffer
pub fn all_json<'a, T>(bytes: &'a [u8]) -> impl Iterator<Item=T> + 'a
    where T: Deserialize<'a> + 'a {
    IterJson::new(bytes)
        .flat_map(|bytes| std::str::from_utf8(bytes).into_iter())
        .flat_map(|x| json5::from_str(x).into_iter())
}


/// Deserializer for fields that can be a number or a string representation of the number
pub fn number_or_string<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr + serde::Deserialize<'de>,
        <T as FromStr>::Err: Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt<T> {
        String(String),
        Number(T),
    }

    match StringOrInt::<T>::deserialize(deserializer)? {
        StringOrInt::String(s) => s.parse::<T>().map_err(serde::de::Error::custom),
        StringOrInt::Number(i) => Ok(i),
    }
}

#[test]
fn test_iterjson() {
    fn f(s: &str) -> Vec<String> {
        IterJson::new(s.as_bytes())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect()
    }
    assert_eq!(f(" { a { b { c } d { e } f {{ g }}   "), vec!["{ c }", "{ e }", "{ g }", "{{ g }}"]);
    assert_eq!(f(r#"{"k":{"k":"v"}}"#), vec![r#"{"k":"v"}"#, r#"{"k":{"k":"v"}}"#]);
    assert_eq!(f(r#"xxx}}xx{{xxx{a}"#), vec!["{a}"]);
    let only_open = String::from_utf8(vec![b'{'; 1000000]).unwrap();
    assert_eq!(f(&only_open), Vec::<String>::new());
}

#[test]
fn test_alljson() {
    #[derive(Deserialize, Debug, PartialEq, Eq)]
    struct S { x: u8 }
    let actual: Vec<S> = all_json(&br#"{{  "x":1}{-}--{{{"x":2}}"#[..]).collect();
    assert_eq!(actual, vec![S { x: 1 }, S { x: 2 }]);
}