//! 只扫 JSON **顶层**键的窥视器。
//!
//! [`serde_json::from_slice`] 即便目标结构只有几个字段，也必须把
//! `messages` / `input` 里的每个字符串都扫一遍（跳过 ≠ 不碰）。火焰图上
//! `peek_request_body` 的 serde 叶子就是这个。
//!
//! 这里按 RFC 8259 走顶层对象：认识的键把**值切片**交给 serde 解
//! （转义 / 数字格式跟 serde 完全一致），不认识的键按括号深度跳过，
//! **不分配、不进 serde**。嵌套里的 `model` / `stream` / `usage` 因此
//! 既不会漏进来，也不会被扫成字符串。

use std::borrow::Cow;

/// serde_json 默认的递归上限。超过就当解析失败，与
/// `serde_json::from_slice` 同一条边界。
const MAX_DEPTH: u32 = 128;

/// 网关必须从顶层读到的字段。语义对齐 `spec::RawPeek`。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TopFields {
    pub model: Option<String>,
    pub stream: Option<bool>,
    pub max_tokens: Option<i64>,
    pub max_completion_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub stream_options_present: bool,
}

/// 顶层对象里名为 `key` 的值的原始 JSON 切片（已去掉值前空白）。
///
/// 输入不是对象、键不存在、或 JSON 不合法时返回 `None`。
/// 重复键取**最后一个**（与 serde_json 一致）。
#[must_use]
pub fn top_level_field<'a>(input: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let mut cur = Cursor::new(input);
    let mut found = None;
    cur.for_each_top_field(|name, value| {
        if name == key {
            found = Some(value);
        }
        Some(())
    })?;
    found
}

/// 只解析网关关心的顶层字段。失败（非对象、类型不对、非法 JSON）返回 `None`，
/// 调用方应把它当成 `body_visible = false` —— 与
/// `serde_json::from_slice::<RawPeek>` 失败时的处理相同。
#[must_use]
pub fn parse_top_fields(input: &[u8]) -> Option<TopFields> {
    let mut out = TopFields::default();
    let mut cur = Cursor::new(input);
    cur.for_each_top_field(|name, value| {
        match name.as_ref() {
            "model" => out.model = Some(serde_json::from_slice(value).ok()?),
            "stream" => out.stream = Some(serde_json::from_slice(value).ok()?),
            "max_tokens" => out.max_tokens = Some(serde_json::from_slice(value).ok()?),
            "max_completion_tokens" => {
                out.max_completion_tokens = Some(serde_json::from_slice(value).ok()?);
            }
            "max_output_tokens" => {
                out.max_output_tokens = Some(serde_json::from_slice(value).ok()?);
            }
            "stream_options" => {
                // 键在就行，值是 null / 对象 / 随便什么都算「写过」。
                out.stream_options_present = true;
            }
            _ => {}
        }
        Some(())
    })?;
    Some(out)
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn new(s: &'a [u8]) -> Self {
        Self { s, i: 0 }
    }

    fn rest(&self) -> &'a [u8] {
        self.s.get(self.i..).unwrap_or_default()
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) {
        self.i = self.i.saturating_add(1);
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.bump();
        }
    }

    fn expect(&mut self, want: u8) -> Option<()> {
        if self.peek() == Some(want) {
            self.bump();
            Some(())
        } else {
            None
        }
    }

    fn eat(&mut self, lit: &[u8]) -> Option<()> {
        if self.rest().starts_with(lit) {
            self.i += lit.len();
            Some(())
        } else {
            None
        }
    }

    /// 遍历顶层对象的每个 `key: value`。`value` 是去掉前导空白后的原始切片。
    fn for_each_top_field(
        &mut self,
        mut visit: impl FnMut(Cow<'a, str>, &'a [u8]) -> Option<()>,
    ) -> Option<()> {
        self.skip_ws();
        self.expect(b'{')?;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.bump();
            self.skip_ws();
            return (self.i == self.s.len()).then_some(());
        }
        loop {
            let name = self.read_key()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let start = self.i;
            self.skip_value(1)?;
            visit(name, &self.s[start..self.i])?;
            self.skip_ws();
            match self.peek()? {
                b',' => {
                    self.bump();
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        return None; // 尾逗号：serde_json 拒
                    }
                }
                b'}' => {
                    self.bump();
                    self.skip_ws();
                    return (self.i == self.s.len()).then_some(());
                }
                _ => return None,
            }
        }
    }

    fn read_key(&mut self) -> Option<Cow<'a, str>> {
        let start = self.i;
        self.skip_string()?;
        let raw = &self.s[start..self.i];
        if raw.contains(&b'\\') {
            let decoded: String = serde_json::from_slice(raw).ok()?;
            Some(Cow::Owned(decoded))
        } else {
            let inner = raw.get(1..raw.len().saturating_sub(1))?;
            Some(Cow::Borrowed(std::str::from_utf8(inner).ok()?))
        }
    }

    fn skip_value(&mut self, depth: u32) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'"' => self.skip_string(),
            b'{' => {
                self.bump();
                self.skip_container(b'}', depth)
            }
            b'[' => {
                self.bump();
                self.skip_container(b']', depth)
            }
            b't' => self.eat(b"true"),
            b'f' => self.eat(b"false"),
            b'n' => self.eat(b"null"),
            b'-' | b'0'..=b'9' => self.skip_number(),
            _ => None,
        }
    }

    fn skip_container(&mut self, close: u8, depth: u32) -> Option<()> {
        self.skip_ws();
        if self.peek() == Some(close) {
            self.bump();
            return Some(());
        }
        loop {
            if close == b'}' {
                self.skip_ws();
                self.skip_string()?;
                self.skip_ws();
                self.expect(b':')?;
                self.skip_ws();
                self.skip_value(depth + 1)?;
            } else {
                self.skip_ws();
                self.skip_value(depth + 1)?;
            }
            self.skip_ws();
            match self.peek()? {
                b',' => {
                    self.bump();
                    self.skip_ws();
                    if self.peek() == Some(close) {
                        return None;
                    }
                }
                b if b == close => {
                    self.bump();
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn skip_string(&mut self) -> Option<()> {
        self.expect(b'"')?;
        loop {
            match self.peek()? {
                b'"' => {
                    self.bump();
                    return Some(());
                }
                b'\\' => {
                    self.bump();
                    self.peek()?;
                    self.bump();
                }
                _ => self.bump(),
            }
        }
    }

    fn skip_number(&mut self) -> Option<()> {
        if self.peek() == Some(b'-') {
            self.bump();
        }
        match self.peek()? {
            b'0' => self.bump(),
            b'1'..=b'9' => {
                self.bump();
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.bump();
                }
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.bump();
            if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                return None;
            }
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.bump();
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump();
            }
            if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                return None;
            }
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.bump();
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests;
