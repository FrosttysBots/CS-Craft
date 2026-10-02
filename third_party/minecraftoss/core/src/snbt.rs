//! Stringified NBT (`TagParser` over 26.3's `SnbtGrammar`), as commands
//! take it: compounds, lists (mixed element types allowed), `[B;`, `[I;`
//! and `[L;` arrays, numbers with their type suffixes (`b`, `s`, `i`, `l`,
//! `f`, `d`) and signedness prefixes (`u`, `s`), hexadecimal (`0x`) and
//! binary (`0b`) integers, underscores between digits, `true` and `false`,
//! quoted strings with escapes, unquoted strings, and the `bool(...)` and
//! `uuid(...)` operations.

use crate::nbt::Tag;
use std::collections::BTreeMap;

/// Parses one SNBT value; trailing text other than whitespace is an error.
pub fn parse(text: &str) -> Result<Tag, String> {
    let mut parser = Parser { chars: text.chars().collect(), at: 0 };
    let tag = parser.literal()?;
    parser.skip_whitespace();
    if parser.at < parser.chars.len() {
        return Err(parser.error("trailing data"));
    }
    Ok(tag)
}

/// Parses an SNBT compound (`CompoundTagArgument`).
pub fn parse_compound(text: &str) -> Result<Tag, String> {
    match parse(text)? {
        tag @ Tag::Compound(_) => Ok(tag),
        _ => Err("expected a compound".to_owned()),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Float,
    Double,
    Byte,
    Short,
    Int,
    Long,
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    fn error(&self, what: &str) -> String {
        format!("{what} at position {}", self.at)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        self.skip_whitespace();
        if self.peek() == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected '{c}'")))
        }
    }

    /// `literal`: a number, a quoted string, a compound, a list or array,
    /// or an unquoted string (maybe a boolean or an operation).
    fn literal(&mut self) -> Result<Tag, String> {
        self.skip_whitespace();
        match self.peek() {
            None => Err(self.error("expected a value")),
            Some('{') => self.compound(),
            Some('[') => self.list(),
            Some('"' | '\'') => Ok(Tag::String(self.quoted()?)),
            Some(c) if can_start_number(c) => {
                let start = self.at;
                if let Some(tag) = self.float()? {
                    return Ok(tag);
                }
                self.at = start;
                self.integer(None)
            }
            Some(_) => self.unquoted_or_builtin(),
        }
    }

    fn compound(&mut self) -> Result<Tag, String> {
        self.expect('{')?;
        let mut map = BTreeMap::new();
        loop {
            self.skip_whitespace();
            if self.peek() == Some('}') {
                self.at += 1;
                return Ok(Tag::Compound(map));
            }
            let key = match self.peek() {
                Some('"' | '\'') => self.quoted()?,
                _ => self.unquoted(),
            };
            if key.is_empty() {
                return Err(self.error("empty key"));
            }
            self.expect(':')?;
            // Later entries win (`buildKeepingLast`).
            map.insert(key, self.literal()?);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => self.at += 1,
                Some('}') => {}
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn list(&mut self) -> Result<Tag, String> {
        self.expect('[')?;
        self.skip_whitespace();
        // A typed array: `[B;`, `[I;` or `[L;`.
        if let (Some(prefix @ ('B' | 'I' | 'L')), Some(';')) = (self.peek(), self.peek_at(1)) {
            self.at += 2;
            let kind = match prefix {
                'B' => Kind::Byte,
                'I' => Kind::Int,
                _ => Kind::Long,
            };
            let mut values = Vec::new();
            loop {
                self.skip_whitespace();
                if self.peek() == Some(']') {
                    self.at += 1;
                    break;
                }
                values.push(self.integer(Some(kind))?);
                self.skip_whitespace();
                match self.peek() {
                    Some(',') => self.at += 1,
                    Some(']') => {}
                    _ => return Err(self.error("expected ',' or ']'")),
                }
            }
            let number = |tag: &Tag| tag.as_i64().unwrap_or(0);
            return Ok(match kind {
                Kind::Byte => Tag::ByteArray(values.iter().map(|v| number(v) as i8).collect()),
                Kind::Int => Tag::IntArray(values.iter().map(|v| number(v) as i32).collect()),
                _ => Tag::LongArray(values.iter().map(number).collect()),
            });
        }
        let mut items = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek() == Some(']') {
                self.at += 1;
                return Ok(Tag::List(items));
            }
            items.push(self.literal()?);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => self.at += 1,
                Some(']') => {}
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    /// A quoted string: either quote, the other quote inside it, and the
    /// escapes `\\`, `\'`, `\"`, `\b`, `\s`, `\t`, `\n`, `\f`, `\r`,
    /// `\xHH`, `\uHHHH` and `\UHHHHHHHH`.
    fn quoted(&mut self) -> Result<String, String> {
        let quote = self.peek().ok_or_else(|| self.error("expected a string"))?;
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else { return Err(self.error("unterminated string")) };
            self.at += 1;
            if c == quote {
                return Ok(out);
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let Some(e) = self.peek() else { return Err(self.error("unterminated escape")) };
            self.at += 1;
            let hex = |parser: &mut Self, digits: usize| -> Result<char, String> {
                let text: String = parser.chars.get(parser.at..parser.at + digits).map(|s| s.iter().collect()).unwrap_or_default();
                parser.at += digits;
                u32::from_str_radix(&text, 16).ok().and_then(char::from_u32).ok_or_else(|| parser.error("invalid escape"))
            };
            out.push(match e {
                '\\' | '\'' | '"' => e,
                'b' => '\u{8}',
                's' => ' ',
                't' => '\t',
                'n' => '\n',
                'f' => '\u{c}',
                'r' => '\r',
                'x' => hex(self, 2)?,
                'u' => hex(self, 4)?,
                'U' => hex(self, 8)?,
                _ => return Err(self.error("invalid escape")),
            });
        }
    }

    /// `UnquotedStringParseRule`: letters, digits and `_-.+`.
    fn unquoted(&mut self) -> String {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')) {
            self.at += 1;
        }
        self.chars[start..self.at].iter().collect()
    }

    fn unquoted_or_builtin(&mut self) -> Result<Tag, String> {
        let text = self.unquoted();
        if text.is_empty() || text.chars().next().is_some_and(can_start_number) {
            return Err(self.error("invalid unquoted string"));
        }
        self.skip_whitespace();
        if self.peek() == Some('(') {
            self.at += 1;
            let argument = self.literal()?;
            self.skip_whitespace();
            if self.peek() == Some(',') {
                self.at += 1;
            }
            self.expect(')')?;
            return match text.as_str() {
                // `bool(x)`: a boolean or a number's non-zero.
                "bool" => match argument {
                    Tag::Byte(v) => Ok(Tag::Byte(i8::from(v != 0))),
                    Tag::Short(v) => Ok(Tag::Byte(i8::from(v != 0))),
                    Tag::Int(v) => Ok(Tag::Byte(i8::from(v != 0))),
                    Tag::Long(v) => Ok(Tag::Byte(i8::from(v != 0))),
                    Tag::Float(v) => Ok(Tag::Byte(i8::from(v != 0.0))),
                    Tag::Double(v) => Ok(Tag::Byte(i8::from(v != 0.0))),
                    _ => Err(self.error("bool() needs a number")),
                },
                // `uuid(str)`: the four ints of `UUIDUtil.uuidToIntArray`.
                "uuid" => match argument {
                    Tag::String(s) => uuid_ints(&s).map(Tag::IntArray).ok_or_else(|| self.error("invalid UUID")),
                    _ => Err(self.error("uuid() needs a string")),
                },
                _ => Err(self.error(&format!("no such operation: {text}"))),
            };
        }
        if text.eq_ignore_ascii_case("true") {
            return Ok(Tag::Byte(1));
        }
        if text.eq_ignore_ascii_case("false") {
            return Ok(Tag::Byte(0));
        }
        Ok(Tag::String(text))
    }

    fn sign(&mut self) -> bool {
        match self.peek() {
            Some('-') => {
                self.at += 1;
                true
            }
            Some('+') => {
                self.at += 1;
                false
            }
            _ => false,
        }
    }

    /// `decimal_numeral`: digits with underscores between them.
    fn digits(&mut self, radix: u32) -> Option<String> {
        let start = self.at;
        if !self.peek().is_some_and(|c| c.is_digit(radix)) {
            return None;
        }
        while self.peek().is_some_and(|c| c.is_digit(radix) || c == '_') {
            self.at += 1;
        }
        let text: String = self.chars[start..self.at].iter().filter(|&&c| c != '_').collect();
        Some(text)
    }

    /// `float_literal`, or `None` (position unspecified) when the text is
    /// not one.
    fn float(&mut self) -> Result<Option<Tag>, String> {
        let minus = self.sign();
        let whole = self.digits(10);
        let mut fraction = None;
        let mut dotted = false;
        if self.peek() == Some('.') {
            dotted = true;
            self.at += 1;
            fraction = self.digits(10);
            if whole.is_none() && fraction.is_none() {
                return Ok(None);
            }
        } else if whole.is_none() {
            return Ok(None);
        }
        let mut exponent = None;
        if matches!(self.peek(), Some('e' | 'E')) {
            let mark = self.at;
            self.at += 1;
            let negative = self.sign();
            match self.digits(10) {
                Some(digits) => exponent = Some((negative, digits)),
                None => self.at = mark,
            }
        }
        let suffix = match self.peek() {
            Some('f' | 'F') => Some(Kind::Float),
            Some('d' | 'D') => Some(Kind::Double),
            _ => None,
        };
        // Whole digits alone need an exponent or a float suffix.
        if !dotted && exponent.is_none() && suffix.is_none() {
            return Ok(None);
        }
        if suffix.is_some() {
            self.at += 1;
        }
        let mut text = String::new();
        if minus {
            text.push('-');
        }
        text.push_str(whole.as_deref().unwrap_or(""));
        if let Some(fraction) = fraction {
            text.push('.');
            text.push_str(&fraction);
        }
        if let Some((negative, digits)) = exponent {
            text.push('e');
            if negative {
                text.push('-');
            }
            text.push_str(&digits);
        }
        if text.starts_with('.') || text.starts_with("-.") {
            text = text.replacen('.', "0.", 1);
        }
        Ok(Some(if suffix == Some(Kind::Float) {
            let value: f32 = text.parse().map_err(|_| self.error("invalid float"))?;
            if !value.is_finite() {
                return Err(self.error("infinity is not allowed"));
            }
            Tag::Float(value)
        } else {
            let value: f64 = text.parse().map_err(|_| self.error("invalid double"))?;
            if !value.is_finite() {
                return Err(self.error("infinity is not allowed"));
            }
            Tag::Double(value)
        }))
    }

    /// `integer_literal` and `IntegerLiteral.create`: decimal numbers are
    /// signed by default, hexadecimal and binary ones unsigned; an array's
    /// entries default to its element type.
    fn integer(&mut self, default_kind: Option<Kind>) -> Result<Tag, String> {
        let minus = self.sign();
        let (radix, digits) = if self.peek() == Some('0') {
            match self.peek_at(1) {
                Some('x' | 'X') => {
                    self.at += 2;
                    (16, self.digits(16).ok_or_else(|| self.error("expected hex digits"))?)
                }
                Some('b' | 'B') if self.peek_at(2).is_some_and(|c| c == '0' || c == '1') => {
                    self.at += 2;
                    (2, self.digits(2).unwrap_or_default())
                }
                Some(c) if c.is_ascii_digit() => return Err(self.error("leading zeros are not allowed")),
                _ => {
                    self.at += 1;
                    (10, "0".to_owned())
                }
            }
        } else {
            (10, self.digits(10).ok_or_else(|| self.error("expected a number"))?)
        };
        // `integer_suffix`: an optional signedness, then the type.
        let mut signed = None;
        let mut kind = None;
        let type_of = |c: Option<char>| match c {
            Some('b' | 'B') => Some(Kind::Byte),
            Some('s' | 'S') => Some(Kind::Short),
            Some('i' | 'I') => Some(Kind::Int),
            Some('l' | 'L') => Some(Kind::Long),
            _ => None,
        };
        match self.peek() {
            Some('u' | 'U') if type_of(self.peek_at(1)).is_some() => {
                signed = Some(false);
                kind = type_of(self.peek_at(1));
                self.at += 2;
            }
            Some('s' | 'S') if type_of(self.peek_at(1)).is_some() => {
                signed = Some(true);
                kind = type_of(self.peek_at(1));
                self.at += 2;
            }
            c => {
                if let Some(k) = type_of(c) {
                    kind = Some(k);
                    self.at += 1;
                }
            }
        }
        let kind = kind.or(default_kind).unwrap_or(Kind::Int);
        let signed = signed.unwrap_or(radix == 10);
        if !signed && minus {
            return Err(self.error("expected a non-negative number"));
        }
        let text = if minus { format!("-{digits}") } else { digits };
        let range = |ok: bool| if ok { Ok(()) } else { Err(self.error("number out of range")) };
        Ok(if signed {
            match kind {
                Kind::Byte => Tag::Byte(i8::from_str_radix(&text, radix).map_err(|_| self.error("number out of range"))?),
                Kind::Short => Tag::Short(i16::from_str_radix(&text, radix).map_err(|_| self.error("number out of range"))?),
                Kind::Long => Tag::Long(i64::from_str_radix(&text, radix).map_err(|_| self.error("number out of range"))?),
                _ => Tag::Int(i32::from_str_radix(&text, radix).map_err(|_| self.error("number out of range"))?),
            }
        } else {
            let value = u64::from_str_radix(&text, radix).map_err(|_| self.error("number out of range"))?;
            match kind {
                Kind::Byte => {
                    range(value <= u64::from(u8::MAX))?;
                    Tag::Byte(value as u8 as i8)
                }
                Kind::Short => {
                    range(value <= u64::from(u16::MAX))?;
                    Tag::Short(value as u16 as i16)
                }
                Kind::Long => Tag::Long(value as i64),
                _ => {
                    range(value <= u64::from(u32::MAX))?;
                    Tag::Int(value as u32 as i32)
                }
            }
        })
    }
}

fn can_start_number(c: char) -> bool {
    matches!(c, '+' | '-' | '.' | '0'..='9')
}

/// `UUID.fromString` as `UUIDUtil.uuidToIntArray`'s four ints.
fn uuid_ints(text: &str) -> Option<Vec<i32>> {
    let parts: Vec<&str> = text.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let field = |s: &str| u64::from_str_radix(s, 16).ok();
    let most = (field(parts[0])? & 0xFFFF_FFFF) << 32 | (field(parts[1])? & 0xFFFF) << 16 | (field(parts[2])? & 0xFFFF);
    let least = (field(parts[3])? & 0xFFFF) << 48 | (field(parts[4])? & 0xFFFF_FFFF_FFFF);
    Some(vec![(most >> 32) as i32, most as i32, (least >> 32) as i32, least as i32])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(tag: &'a Tag, key: &str) -> &'a Tag {
        tag.get(key).unwrap_or_else(|| panic!("missing {key}"))
    }

    #[test]
    fn numbers_take_their_suffixes() {
        let tag = parse_compound("{a:1b,b:2s,c:3,d:4L,e:1.5f,f:2.5,g:3d,h:-7,i:1e3,j:.5,k:0xFFub,l:0b101,m:1_000,n:5sb,o:0,p:true,q:False}").unwrap();
        assert_eq!(get(&tag, "a"), &Tag::Byte(1));
        assert_eq!(get(&tag, "b"), &Tag::Short(2));
        assert_eq!(get(&tag, "c"), &Tag::Int(3));
        assert_eq!(get(&tag, "d"), &Tag::Long(4));
        assert_eq!(get(&tag, "e"), &Tag::Float(1.5));
        assert_eq!(get(&tag, "f"), &Tag::Double(2.5));
        assert_eq!(get(&tag, "g"), &Tag::Double(3.0));
        assert_eq!(get(&tag, "h"), &Tag::Int(-7));
        assert_eq!(get(&tag, "i"), &Tag::Double(1000.0));
        assert_eq!(get(&tag, "j"), &Tag::Double(0.5));
        assert_eq!(get(&tag, "k"), &Tag::Byte(-1));
        assert_eq!(get(&tag, "l"), &Tag::Int(5));
        assert_eq!(get(&tag, "m"), &Tag::Int(1000));
        assert_eq!(get(&tag, "n"), &Tag::Byte(5));
        assert_eq!(get(&tag, "o"), &Tag::Int(0));
        assert_eq!(get(&tag, "p"), &Tag::Byte(1));
        assert_eq!(get(&tag, "q"), &Tag::Byte(0));
        assert!(parse("300b").is_err());
        assert!(parse("-0x1").is_err());
        assert!(parse("012").is_err());
    }

    #[test]
    fn structures_strings_and_operations() {
        let tag = parse_compound(
            r#"{ UUID:[I;0,0,0,1201], Tags:["zombie", 'it''s'], Pos:[1.0d,2.5,-3d], "odd key":"a\"b\n", Name:plain_word-1.0, Data:{level:1,type:"minecraft:desert"}, Empty:[], B:[B;1b,2,3], L:[L;1,2l], Flag:bool(2), Id:uuid("00000000-0000-0001-0000-000000000002") }"#,
        );
        // `'it''s'` is not valid: a single-quoted string ends at its quote.
        assert!(tag.is_err());
        let tag = parse_compound(
            r#"{ UUID:[I;0,0,0,1201], Tags:["zombie", 'it"s'], Pos:[1.0d,2.5,-3d], "odd key":"a\"b\n", Name:plain_word-1.0, Data:{level:1,type:"minecraft:desert"}, Empty:[], B:[B;1b,2,3], L:[L;1,2l], Flag:bool(2), Id:uuid("00000000-0000-0001-0000-000000000002") }"#,
        )
        .unwrap();
        assert_eq!(get(&tag, "UUID"), &Tag::IntArray(vec![0, 0, 0, 1201]));
        assert_eq!(get(&tag, "Tags"), &Tag::List(vec![Tag::String("zombie".into()), Tag::String("it\"s".into())]));
        assert_eq!(get(&tag, "Pos"), &Tag::List(vec![Tag::Double(1.0), Tag::Double(2.5), Tag::Double(-3.0)]));
        assert_eq!(get(&tag, "odd key"), &Tag::String("a\"b\n".into()));
        assert_eq!(get(&tag, "Name"), &Tag::String("plain_word-1.0".into()));
        assert_eq!(get(get(&tag, "Data"), "type"), &Tag::String("minecraft:desert".into()));
        assert_eq!(get(&tag, "Empty"), &Tag::List(vec![]));
        assert_eq!(get(&tag, "B"), &Tag::ByteArray(vec![1, 2, 3]));
        assert_eq!(get(&tag, "L"), &Tag::LongArray(vec![1, 2]));
        assert_eq!(get(&tag, "Flag"), &Tag::Byte(1));
        assert_eq!(get(&tag, "Id"), &Tag::IntArray(vec![0, 1, 0, 2]));
        assert!(parse_compound("{a:1} x").is_err());
        assert!(parse_compound("[1,2]").is_err());
    }
}
