//! KeyValues ("VDF") text parser.
//!
//! Source engine text data (`gameinfo.txt`, `items_game.txt`, `.vmt`
//! materials, the BSP entity lump) is a tree of `"key" "value"` pairs and
//! `"key" { ... }` blocks. Keys may repeat and compare case-insensitively.
//!
//! Supported syntax:
//! * quoted and bare tokens; `\n`, `\t`, `\\` and `\"` escapes only when
//!   asked for ([`parse_escaped`]), because the engine leaves them off by
//!   default and paths such as `tools\toolsblack` rely on that
//! * `//` line comments
//! * platform conditionals such as `[$WIN32]` or `[!$X360 && !$PS3]`,
//!   evaluated for a Windows PC
//! * UTF-8 and UTF-16LE input (with BOM), as used by localisation files
//!
//! `#base` / `#include` directives are returned as ordinary entries named
//! `#base` / `#include`; resolving them needs a file system and is the
//! caller's job.

use std::fmt;

/// A value: either a string or a nested block.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Block(Block),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Block(_) => None,
        }
    }

    pub fn as_block(&self) -> Option<&Block> {
        match self {
            Value::Block(b) => Some(b),
            Value::Str(_) => None,
        }
    }

    pub fn as_block_mut(&mut self) -> Option<&mut Block> {
        match self {
            Value::Block(b) => Some(b),
            Value::Str(_) => None,
        }
    }
}

/// An ordered list of entries. Duplicate keys are kept in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub entries: Vec<(String, Value)>,
}

impl Block {
    pub fn new() -> Self {
        Self::default()
    }

    /// First value whose key matches, ignoring ASCII case.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    /// All values whose key matches, ignoring ASCII case.
    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.entries
            .iter()
            .filter(move |(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn block(&self, key: &str) -> Option<&Block> {
        self.get(key).and_then(Value::as_block)
    }

    /// Follows a `/`-free path of nested block keys, e.g. `["GameInfo", "FileSystem"]`.
    pub fn path(&self, keys: &[&str]) -> Option<&Value> {
        let (last, parents) = keys.split_last()?;
        let mut cur = self;
        for k in parents {
            cur = cur.block(k)?;
        }
        cur.get(last)
    }

    pub fn f32(&self, key: &str) -> Option<f32> {
        self.str(key).and_then(parse_f32)
    }

    pub fn i64(&self, key: &str) -> Option<i64> {
        self.str(key).and_then(parse_i64)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.i64(key).map(|v| v != 0)
    }

    /// Replaces the first entry with this key, or appends one.
    pub fn set(&mut self, key: &str, value: Value) {
        match self.get_mut(key) {
            Some(slot) => *slot = value,
            None => self.entries.push((key.to_string(), value)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// Lenient float parse in the style of `atof`: leading numeric prefix, else `None`.
pub fn parse_f32(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Ok(v) = s.parse::<f32>() {
        return Some(v);
    }
    let end = s
        .char_indices()
        .take_while(|(i, c)| {
            c.is_ascii_digit() || *c == '.' || ((*c == '-' || *c == '+') && *i == 0)
        })
        .map(|(i, c)| i + c.len_utf8())
        .last()?;
    s[..end].parse().ok()
}

/// Lenient integer parse in the style of `atoi`.
pub fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(v) = s.parse::<i64>() {
        return Some(v);
    }
    parse_f32(s).map(|f| f as i64)
}

/// Parses whitespace-separated floats, tolerating `[ ]` and `{ }` wrappers
/// as used by material vectors (`"[1 0.5 0]"`).
pub fn parse_vec(s: &str) -> Vec<f32> {
    s.split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '{' | '}'))
        .filter(|t| !t.is_empty())
        .filter_map(parse_f32)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Decodes raw file bytes to text, handling UTF-8/UTF-16LE BOMs.
pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Parses a whole document into a root block (the implicit top level).
/// Backslashes are literal, as in the engine's default mode.
pub fn parse(text: &str) -> Result<Block, ParseError> {
    let mut lexer = Lexer::new(text, false);
    parse_block(&mut lexer, true)
}

/// Like [`parse`], but interprets `\n`, `\t`, `\\` and `\"` inside quotes
/// (localisation files are written this way).
pub fn parse_escaped(text: &str) -> Result<Block, ParseError> {
    let mut lexer = Lexer::new(text, true);
    parse_block(&mut lexer, true)
}

/// Parses raw bytes, decoding UTF-8 or UTF-16 first.
pub fn parse_bytes(bytes: &[u8]) -> Result<Block, ParseError> {
    parse(&decode_text(bytes))
}

/// Parses the BSP entity lump: a sequence of anonymous `{ ... }` blocks.
pub fn parse_entities(text: &str) -> Result<Vec<Block>, ParseError> {
    let mut lexer = Lexer::new(text, false);
    let mut out = Vec::new();
    loop {
        match lexer.next()? {
            None => break,
            Some(Token::Open) => out.push(parse_block(&mut lexer, false)?),
            Some(Token::Str(_)) | Some(Token::Cond(_)) => {}
            Some(Token::Close) => {
                return Err(lexer.error("unexpected '}' in entity list"));
            }
        }
    }
    Ok(out)
}

fn parse_block(lexer: &mut Lexer, top_level: bool) -> Result<Block, ParseError> {
    let mut block = Block::new();
    loop {
        let key = match lexer.next()? {
            None if top_level => return Ok(block),
            None => return Err(lexer.error("unexpected end of file inside block")),
            Some(Token::Close) if !top_level => return Ok(block),
            // Stray closing braces at the top level show up in hand-edited
            // Valve files; ignore them as the engine does.
            Some(Token::Close) => continue,
            Some(Token::Cond(_)) => continue,
            Some(Token::Open) => {
                // Anonymous block with no key: keep it under an empty key.
                let child = parse_block(lexer, false)?;
                block.entries.push((String::new(), Value::Block(child)));
                continue;
            }
            Some(Token::Str(s)) => s,
        };
        let value = match lexer.next()? {
            None => return Err(lexer.error(&format!("key '{key}' has no value"))),
            Some(Token::Open) => Value::Block(parse_block(lexer, false)?),
            Some(Token::Str(s)) => Value::Str(s),
            Some(Token::Close) => {
                return Err(lexer.error(&format!("key '{key}' followed by '}}'")));
            }
            Some(Token::Cond(_)) => {
                return Err(lexer.error(&format!("key '{key}' followed by a conditional")));
            }
        };
        // Optional trailing conditional decides whether the pair counts.
        let keep = match lexer.peek_cond()? {
            Some(expr) => eval_condition(&expr),
            None => true,
        };
        if keep {
            block.entries.push((key, value));
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
    Cond(String),
}

struct Lexer<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
    line: usize,
    escapes: bool,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str, escapes: bool) -> Self {
        Self {
            src: text.as_bytes(),
            text,
            pos: 0,
            line: 1,
            escapes,
        }
    }

    fn error(&self, message: &str) -> ParseError {
        ParseError {
            line: self.line,
            message: message.to_string(),
        }
    }

    fn skip_ws_and_comments(&mut self) {
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c == b'\n' {
                self.line += 1;
                self.pos += 1;
            } else if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if c == b'/' && self.src.get(self.pos + 1) == Some(&b'/') {
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Consumes a conditional only if it is the next token on the same line.
    fn peek_cond(&mut self) -> Result<Option<String>, ParseError> {
        let save = (self.pos, self.line);
        // Conditionals must follow on the same line.
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c == b' ' || c == b'\t' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.src.get(self.pos) == Some(&b'[') {
            if let Some(Token::Cond(c)) = self.next()? {
                return Ok(Some(c));
            }
        }
        (self.pos, self.line) = save;
        Ok(None)
    }

    fn next(&mut self) -> Result<Option<Token>, ParseError> {
        self.skip_ws_and_comments();
        if self.pos >= self.src.len() {
            return Ok(None);
        }
        let c = self.src[self.pos];
        match c {
            b'{' => {
                self.pos += 1;
                Ok(Some(Token::Open))
            }
            b'}' => {
                self.pos += 1;
                Ok(Some(Token::Close))
            }
            b'[' => {
                let start = self.pos + 1;
                while self.pos < self.src.len() && self.src[self.pos] != b']' {
                    if self.src[self.pos] == b'\n' {
                        self.line += 1;
                    }
                    self.pos += 1;
                }
                let expr = self.text[start..self.pos.min(self.src.len())].to_string();
                self.pos = (self.pos + 1).min(self.src.len());
                Ok(Some(Token::Cond(expr)))
            }
            b'"' => {
                self.pos += 1;
                let mut out = String::new();
                let mut run_start = self.pos;
                loop {
                    if self.pos >= self.src.len() {
                        // Unterminated string at EOF: take what we have.
                        out.push_str(&self.text[run_start..self.pos]);
                        return Ok(Some(Token::Str(out)));
                    }
                    let ch = self.src[self.pos];
                    if ch == b'"' {
                        out.push_str(&self.text[run_start..self.pos]);
                        self.pos += 1;
                        return Ok(Some(Token::Str(out)));
                    }
                    if ch == b'\\' && self.escapes {
                        let esc = self.src.get(self.pos + 1).copied();
                        let rep = match esc {
                            Some(b'n') => Some('\n'),
                            Some(b't') => Some('\t'),
                            Some(b'\\') => Some('\\'),
                            Some(b'"') => Some('"'),
                            _ => None,
                        };
                        if let Some(r) = rep {
                            out.push_str(&self.text[run_start..self.pos]);
                            out.push(r);
                            self.pos += 2;
                            run_start = self.pos;
                            continue;
                        }
                    }
                    if ch == b'\n' {
                        self.line += 1;
                    }
                    self.pos += 1;
                }
            }
            _ => {
                let start = self.pos;
                while self.pos < self.src.len() {
                    let ch = self.src[self.pos];
                    if ch.is_ascii_whitespace() || ch == b'{' || ch == b'}' || ch == b'"' {
                        break;
                    }
                    if ch == b'/' && self.src.get(self.pos + 1) == Some(&b'/') {
                        break;
                    }
                    self.pos += 1;
                }
                Ok(Some(Token::Str(self.text[start..self.pos].to_string())))
            }
        }
    }
}

/// Evaluates a platform conditional for a Windows PC build.
fn eval_condition(expr: &str) -> bool {
    // Grammar: or := and ('||' and)* ; and := term ('&&' term)* ; term := '!'? '$'NAME
    expr.split("||").any(|conj| {
        conj.split("&&").all(|term| {
            let term = term.trim();
            let (neg, name) = match term.strip_prefix('!') {
                Some(rest) => (true, rest.trim()),
                None => (false, term),
            };
            let name = name.trim_start_matches('$').to_ascii_uppercase();
            let value = matches!(name.as_str(), "WIN32" | "WINDOWS" | "PC" | "WIN64");
            value != neg
        })
    })
}

/// Serialises a block back to KeyValues text (tabs, quoted).
pub fn to_string(block: &Block) -> String {
    fn write(out: &mut String, b: &Block, depth: usize) {
        for (k, v) in &b.entries {
            for _ in 0..depth {
                out.push('\t');
            }
            match v {
                Value::Str(s) => {
                    out.push_str(&format!("\"{}\"\t\"{}\"\n", escape(k), escape(s)));
                }
                Value::Block(child) => {
                    out.push_str(&format!("\"{}\"\n", escape(k)));
                    for _ in 0..depth {
                        out.push('\t');
                    }
                    out.push_str("{\n");
                    write(out, child, depth + 1);
                    for _ in 0..depth {
                        out.push('\t');
                    }
                    out.push_str("}\n");
                }
            }
        }
    }
    fn escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
    }
    let mut out = String::new();
    write(&mut out, block, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_blocks_and_case_insensitive_lookup() {
        let doc = parse(
            r#"
            "GameInfo"
            {
                game "Counter-Strike: Global Offensive" // comment
                FileSystem { SearchPaths { Game |gameinfo_path|. Game csgo } }
            }
            "#,
        )
        .unwrap();
        let gi = doc.block("gameinfo").unwrap();
        assert_eq!(gi.str("GAME"), Some("Counter-Strike: Global Offensive"));
        let sp = doc
            .path(&["GameInfo", "FileSystem", "SearchPaths"])
            .unwrap()
            .as_block()
            .unwrap();
        let games: Vec<_> = sp.get_all("game").filter_map(Value::as_str).collect();
        assert_eq!(games, ["|gameinfo_path|.", "csgo"]);
    }

    #[test]
    fn escapes_and_conditionals() {
        let doc = parse_escaped(
            "\"a\" \"x\\\"y\\n\"\n\"b\" \"pc\" [$WIN32]\n\"b\" \"console\" [$X360]\n\"c\" \"1\" [!$X360 && !$PS3]\n",
        )
        .unwrap();
        assert_eq!(doc.str("a"), Some("x\"y\n"));
        let bs: Vec<_> = doc.get_all("b").filter_map(Value::as_str).collect();
        assert_eq!(bs, ["pc"]);
        assert_eq!(doc.i64("c"), Some(1));
    }

    #[test]
    fn backslashes_are_literal_by_default() {
        let doc = parse("\"$basetexture\" \"tools\\toolsblack\"").unwrap();
        assert_eq!(doc.str("$basetexture"), Some("tools\\toolsblack"));
    }

    #[test]
    fn entity_lump() {
        let ents = parse_entities(
            "{\n\"classname\" \"worldspawn\"\n\"skyname\" \"sky_dust\"\n}\n{\n\"classname\" \"info_player_terrorist\"\n\"origin\" \"1 2 3\"\n}\n\0",
        )
        .unwrap();
        assert_eq!(ents.len(), 2);
        assert_eq!(ents[1].str("origin"), Some("1 2 3"));
        assert_eq!(parse_vec(ents[1].str("origin").unwrap()), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn utf16_bom() {
        let text = "\"lang\" { \"Tokens\" { \"x\" \"y\" } }";
        let mut bytes = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let doc = parse_bytes(&bytes).unwrap();
        assert_eq!(doc.path(&["lang", "Tokens", "x"]).unwrap().as_str(), Some("y"));
    }

    #[test]
    fn lenient_numbers() {
        assert_eq!(parse_f32("0.25f"), Some(0.25));
        assert_eq!(parse_i64("12abc"), Some(12));
        assert_eq!(parse_vec("[1 .5 0]"), [1.0, 0.5, 0.0]);
    }
}
