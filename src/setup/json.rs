use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    String(Vec<u8>),
    Array(Vec<Value>),
    Object(Vec<(Vec<u8>, Value)>),
}

impl Value {
    pub fn object() -> Self {
        Self::Object(Vec::new())
    }

    pub fn get(&self, key: &[u8]) -> Option<&Self> {
        let Self::Object(fields) = self else {
            return None;
        };
        fields
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value)
    }

    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Self> {
        let Self::Object(fields) = self else {
            return None;
        };
        fields
            .iter_mut()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value)
    }

    pub fn insert(&mut self, key: &[u8], value: Self) -> Result<(), Error> {
        let Self::Object(fields) = self else {
            return Err(Error);
        };
        if let Some((_, current)) = fields.iter_mut().find(|(candidate, _)| candidate == key) {
            *current = value;
        } else {
            fields.push((key.to_vec(), value));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid JSON")
    }
}

pub fn parse(input: &[u8]) -> Result<Value, Error> {
    let mut parser = Parser { input, position: 0 };
    let value = parser.value()?;
    parser.whitespace();
    (parser.position == input.len())
        .then_some(value)
        .ok_or(Error)
}

pub fn serialize(value: &Value) -> Vec<u8> {
    let mut output = Vec::new();
    write_value(value, &mut output);
    output
}

fn write_value(value: &Value, output: &mut Vec<u8>) {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Integer(integer) => output.extend_from_slice(integer.to_string().as_bytes()),
        Value::String(string) => write_string(string, output),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_value(value, output);
            }
            output.push(b']');
        }
        Value::Object(fields) => {
            output.push(b'{');
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_string(key, output);
                output.push(b':');
                write_value(value, output);
            }
            output.push(b'}');
        }
    }
}

pub fn write_string(input: &[u8], output: &mut Vec<u8>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output.push(b'"');
    for byte in input {
        match byte {
            b'"' => output.extend_from_slice(b"\\\""),
            b'\\' => output.extend_from_slice(b"\\\\"),
            b'\x08' => output.extend_from_slice(b"\\b"),
            b'\x0c' => output.extend_from_slice(b"\\f"),
            b'\n' => output.extend_from_slice(b"\\n"),
            b'\r' => output.extend_from_slice(b"\\r"),
            b'\t' => output.extend_from_slice(b"\\t"),
            0x00..=0x1f => {
                output.extend_from_slice(b"\\u00");
                output.push(HEX[(byte >> 4) as usize]);
                output.push(HEX[(byte & 0x0f) as usize]);
            }
            _ => output.push(*byte),
        }
    }
    output.push(b'"');
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn value(&mut self) -> Result<Value, Error> {
        self.whitespace();
        match self.peek().ok_or(Error)? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Value::String),
            b't' => self.literal(b"true", Value::Bool(true)),
            b'f' => self.literal(b"false", Value::Bool(false)),
            b'n' => self.literal(b"null", Value::Null),
            b'-' | b'0'..=b'9' => self.integer(),
            _ => Err(Error),
        }
    }

    fn object(&mut self) -> Result<Value, Error> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        self.whitespace();
        if self.take(b'}') {
            return Ok(Value::Object(fields));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            self.expect(b':')?;
            let value = self.value()?;
            if let Some((_, current)) = fields
                .iter_mut()
                .find(|(candidate, _): &&mut (Vec<u8>, Value)| *candidate == key)
            {
                *current = value;
            } else {
                fields.push((key, value));
            }
            self.whitespace();
            if self.take(b'}') {
                return Ok(Value::Object(fields));
            }
            self.expect(b',')?;
        }
    }

    fn array(&mut self) -> Result<Value, Error> {
        self.expect(b'[')?;
        let mut values = Vec::new();
        self.whitespace();
        if self.take(b']') {
            return Ok(Value::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.whitespace();
            if self.take(b']') {
                return Ok(Value::Array(values));
            }
            self.expect(b',')?;
        }
    }

    fn string(&mut self) -> Result<Vec<u8>, Error> {
        self.expect(b'"')?;
        let mut value = Vec::new();
        loop {
            let byte = self.next().ok_or(Error)?;
            match byte {
                b'"' => return Ok(value),
                b'\\' => match self.next().ok_or(Error)? {
                    b'"' => value.push(b'"'),
                    b'\\' => value.push(b'\\'),
                    b'/' => value.push(b'/'),
                    b'b' => value.push(0x08),
                    b'f' => value.push(0x0c),
                    b'n' => value.push(b'\n'),
                    b'r' => value.push(b'\r'),
                    b't' => value.push(b'\t'),
                    b'u' => self.unicode_escape(&mut value)?,
                    _ => return Err(Error),
                },
                0x00..=0x1f => return Err(Error),
                _ => value.push(byte),
            }
        }
    }

    fn unicode_escape(&mut self, output: &mut Vec<u8>) -> Result<(), Error> {
        let first = self.hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            self.expect(b'\\')?;
            self.expect(b'u')?;
            let second = self.hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(Error);
            }
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(Error);
        } else {
            u32::from(first)
        };
        let character = char::from_u32(scalar).ok_or(Error)?;
        let mut encoded = [0_u8; 4];
        output.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        Ok(())
    }

    fn hex_quad(&mut self) -> Result<u16, Error> {
        let mut value = 0_u16;
        for _ in 0..4 {
            value = value
                .checked_mul(16)
                .and_then(|current| {
                    self.next()?
                        .to_digit(16)
                        .map(|digit| current + digit as u16)
                })
                .ok_or(Error)?;
        }
        Ok(value)
    }

    fn integer(&mut self) -> Result<Value, Error> {
        let start = self.position;
        self.take(b'-');
        match self.next().ok_or(Error)? {
            b'0' if self.peek().is_some_and(|byte| byte.is_ascii_digit()) => return Err(Error),
            b'0' => {}
            b'1'..=b'9' => {
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.position += 1;
                }
            }
            _ => return Err(Error),
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(Error);
        }
        let text = std::str::from_utf8(&self.input[start..self.position]).map_err(|_| Error)?;
        text.parse::<i64>().map(Value::Integer).map_err(|_| Error)
    }

    fn literal(&mut self, literal: &[u8], value: Value) -> Result<Value, Error> {
        if self.input.get(self.position..self.position + literal.len()) != Some(literal) {
            return Err(Error);
        }
        self.position += literal.len();
        Ok(value)
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), Error> {
        self.take(expected).then_some(()).ok_or(Error)
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.peek() != Some(expected) {
            return false;
        }
        self.position += 1;
        true
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }
}

trait HexDigit {
    fn to_digit(self, radix: u32) -> Option<u32>;
}

impl HexDigit for u8 {
    fn to_digit(self, radix: u32) -> Option<u32> {
        char::from(self).to_digit(radix)
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, parse, serialize};

    #[test]
    fn round_trips_nested_json_and_unicode_escapes() {
        let input =
            br#"{"name":"Tapas \ud83e\uded2","enabled":true,"n":-2,"items":[null,{"x":"a\n"}]}"#;
        let parsed = parse(input).unwrap();
        assert_eq!(parse(&serialize(&parsed)).unwrap(), parsed);
        assert_eq!(
            parsed.get(b"name"),
            Some(&Value::String("Tapas 🫒".as_bytes().to_vec()))
        );
    }

    #[test]
    fn rejects_trailing_content_floats_and_malformed_strings() {
        for input in [
            b"{} trailing".as_slice(),
            b"1.5",
            b"\"unterminated",
            b"\"\\ud800x\"",
        ] {
            assert!(parse(input).is_err(), "{input:?}");
        }
    }
}
