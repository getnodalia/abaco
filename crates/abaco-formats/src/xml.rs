// SPDX-License-Identifier: MIT OR Apache-2.0
//! A small XML reader and writer for the files of Simulink (`.slx` parts): elements,
//! attributes, text, CDATA, comments and processing instructions, with the five predefined
//! entities and character references. Document type declarations are skipped and no other
//! entity is ever expanded, so nothing here can be made to grow by a hostile file.

use crate::error::{corrupt, limit, Result};

/// The deepest element nesting accepted.
pub const MAX_DEPTH: usize = 256;
/// The most nodes a document may hold.
pub const MAX_NODES: usize = 4_000_000;

/// A child of an element.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// A nested element.
    Element(Element),
    /// Character data.
    Text(String),
}

/// An element with its attributes and children.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Element {
    /// The tag name.
    pub name: String,
    /// Attributes in document order.
    pub attrs: Vec<(String, String)>,
    /// Children in document order.
    pub children: Vec<Node>,
}

impl Element {
    /// An element with no attributes or children.
    pub fn new(name: &str) -> Element {
        Element {
            name: name.to_string(),
            ..Element::default()
        }
    }

    /// Sets an attribute and returns the element.
    pub fn with_attr(mut self, k: &str, v: &str) -> Element {
        self.attrs.push((k.to_string(), v.to_string()));
        self
    }

    /// An element whose only child is `text`.
    pub fn with_text(name: &str, text: &str) -> Element {
        let mut e = Element::new(name);
        e.children.push(Node::Text(text.to_string()));
        e
    }

    /// The value of the attribute `k`.
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }

    /// The child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The child elements called `name`.
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.elements().filter(move |e| e.name == name)
    }

    /// The first child element called `name`.
    pub fn child<'a>(&'a self, name: &'a str) -> Option<&'a Element> {
        self.named(name).next()
    }

    /// The text directly inside the element.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for c in &self.children {
            if let Node::Text(t) = c {
                s.push_str(t);
            }
        }
        s
    }

    /// Adds a child element.
    pub fn push(&mut self, e: Element) {
        self.children.push(Node::Element(e));
    }

    /// The document text, indented, with an XML declaration.
    pub fn to_document(&self) -> String {
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        write_element(self, 0, &mut out);
        out
    }
}

fn escape(s: &str, attr: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\n' if attr => out.push_str("&#10;"),
            '\t' if attr => out.push_str("&#9;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
}

fn write_element(e: &Element, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    out.push_str(&pad);
    out.push('<');
    out.push_str(&e.name);
    for (k, v) in &e.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape(v, true, out);
        out.push('"');
    }
    if e.children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    let only_text = e.children.iter().all(|c| matches!(c, Node::Text(_)));
    out.push('>');
    if only_text {
        escape(&e.text(), false, out);
    } else {
        out.push('\n');
        for c in &e.children {
            match c {
                Node::Element(ch) => write_element(ch, depth + 1, out),
                Node::Text(t) if t.trim().is_empty() => {}
                Node::Text(t) => {
                    out.push_str(&pad);
                    out.push_str("  ");
                    escape(t, false, out);
                    out.push('\n');
                }
            }
        }
        out.push_str(&pad);
    }
    out.push_str("</");
    out.push_str(&e.name);
    out.push_str(">\n");
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    nodes: usize,
}

impl<'a> Parser<'a> {
    fn line(&self) -> usize {
        1 + self.s.iter().take(self.i).filter(|&&b| b == b'\n').count()
    }

    fn err<T>(&self, msg: &str) -> Result<T> {
        corrupt(format!("invalid XML at line {}: {msg}", self.line()))
    }

    fn starts(&self, p: &str) -> bool {
        self.s
            .get(self.i..)
            .is_some_and(|r| r.starts_with(p.as_bytes()))
    }

    fn skip_ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn skip_until(&mut self, end: &str) -> Result<()> {
        match self.s.get(self.i..).and_then(|r| find(r, end.as_bytes())) {
            Some(p) => {
                self.i += p + end.len();
                Ok(())
            }
            None => self.err(&format!("`{end}` is missing")),
        }
    }

    fn name(&mut self) -> Result<String> {
        let start = self.i;
        while let Some(&b) = self.s.get(self.i) {
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'-' | b'.') || b >= 0x80 {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.i == start {
            return self.err("a name was expected");
        }
        Ok(self.text.get(start..self.i).unwrap_or("").to_string())
    }

    fn decode(&self, raw: &str) -> Result<String> {
        if !raw.contains('&') {
            return Ok(raw.to_string());
        }
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(p) = rest.find('&') {
            out.push_str(rest.get(..p).unwrap_or(""));
            let after = rest.get(p + 1..).unwrap_or("");
            let Some(semi) = after.find(';').filter(|&s| s <= 10) else {
                return self.err("an `&` is not a valid entity");
            };
            let ent = after.get(..semi).unwrap_or("");
            match ent {
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "amp" => out.push('&'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                e if e.starts_with("#x") || e.starts_with("#X") => {
                    match u32::from_str_radix(e.get(2..).unwrap_or(""), 16)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        Some(c) => out.push(c),
                        None => return self.err("a character reference is invalid"),
                    }
                }
                e if e.starts_with('#') => match e
                    .get(1..)
                    .unwrap_or("")
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(c) => out.push(c),
                    None => return self.err("a character reference is invalid"),
                },
                other => return self.err(&format!("the entity `&{other};` is not defined")),
            }
            rest = after.get(semi + 1..).unwrap_or("");
        }
        out.push_str(rest);
        Ok(out)
    }

    fn misc(&mut self) -> Result<()> {
        loop {
            self.skip_ws();
            if self.starts("<?") {
                self.skip_until("?>")?;
            } else if self.starts("<!--") {
                self.skip_until("-->")?;
            } else if self.starts("<!DOCTYPE") {
                // Skip, honouring one level of internal subset; entities are never expanded.
                let mut depth = 0i32;
                while let Some(&b) = self.s.get(self.i) {
                    self.i += 1;
                    match b {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => break,
                        _ => {}
                    }
                }
            } else {
                return Ok(());
            }
        }
    }

    fn element(&mut self) -> Result<Element> {
        let mut stack: Vec<Element> = Vec::new();
        loop {
            if !self.starts("<") {
                return self.err("an element was expected");
            }
            self.i += 1;
            let name = self.name()?;
            let mut el = Element::new(&name);
            loop {
                self.skip_ws();
                match self.s.get(self.i) {
                    Some(b'/') | Some(b'>') => break,
                    Some(_) => {
                        let k = self.name()?;
                        self.skip_ws();
                        if self.s.get(self.i) != Some(&b'=') {
                            return self.err("`=` was expected after an attribute name");
                        }
                        self.i += 1;
                        self.skip_ws();
                        let q = match self.s.get(self.i) {
                            Some(&q) if q == b'"' || q == b'\'' => q,
                            _ => return self.err("an attribute value must be quoted"),
                        };
                        self.i += 1;
                        let start = self.i;
                        while self.s.get(self.i).is_some_and(|&b| b != q) {
                            self.i += 1;
                        }
                        if self.i >= self.s.len() {
                            return self.err("an attribute value is not closed");
                        }
                        let raw = self.text.get(start..self.i).unwrap_or("");
                        self.i += 1;
                        let v = self.decode(raw)?;
                        el.attrs.push((k, v));
                    }
                    None => return self.err("the document ends inside a tag"),
                }
            }
            self.nodes += 1;
            if self.nodes > MAX_NODES {
                return limit("the document has too many nodes");
            }
            let mut finished = if self.starts("/>") {
                self.i += 2;
                Some(el)
            } else {
                self.i += 1;
                stack.push(el);
                if stack.len() > MAX_DEPTH {
                    return limit("elements are nested too deeply");
                }
                None
            };
            // Content of the innermost open element, until the next child start or the end.
            loop {
                if let Some(done) = finished.take() {
                    match stack.last_mut() {
                        Some(parent) => parent.push(done),
                        None => return Ok(done),
                    }
                }
                let Some(top) = stack.last_mut() else {
                    return self.err("unbalanced tags");
                };
                if self.i >= self.s.len() {
                    return self.err(&format!("`<{}>` is never closed", top.name));
                }
                if self.starts("<![CDATA[") {
                    let start = self.i + 9;
                    self.skip_until("]]>")?;
                    let t = self.text.get(start..self.i - 3).unwrap_or("").to_string();
                    top.children.push(Node::Text(t));
                } else if self.starts("<!--") {
                    self.skip_until("-->")?;
                } else if self.starts("<?") {
                    self.skip_until("?>")?;
                } else if self.starts("</") {
                    self.i += 2;
                    let close = self.name()?;
                    self.skip_ws();
                    if self.s.get(self.i) != Some(&b'>') {
                        return self.err("`>` was expected");
                    }
                    self.i += 1;
                    if let Some(open) = stack.pop() {
                        if open.name != close {
                            return self.err(&format!("`</{close}>` closes `<{}>`", open.name));
                        }
                        finished = Some(open);
                    }
                } else if self.starts("<") {
                    break;
                } else {
                    let start = self.i;
                    while self.s.get(self.i).is_some_and(|&b| b != b'<') {
                        self.i += 1;
                    }
                    let raw = self.text.get(start..self.i).unwrap_or("");
                    let t = self.decode(raw)?;
                    if let Some(top) = stack.last_mut() {
                        top.children.push(Node::Text(t));
                    }
                }
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Parses a document and returns its root element.
pub fn parse(text: &str) -> Result<Element> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
        nodes: 0,
    };
    p.misc()?;
    let root = p.element()?;
    p.misc()?;
    if p.i < p.s.len() {
        return p.err("there is content after the root element");
    }
    Ok(root)
}

/// Parses a document given as bytes (UTF-8, optionally with a byte order mark).
pub fn parse_bytes(b: &[u8]) -> Result<Element> {
    match std::str::from_utf8(b) {
        Ok(s) => parse(s),
        Err(_) => corrupt("the XML is not valid UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes() {
        let doc = r#"<?xml version="1.0"?>
<!-- c --><a x="1 &amp; 2" y='q'><b>t &lt;1&gt; &#65;&#x42;</b><c/><![CDATA[raw <>]]><d><e k="v"/></d></a>"#;
        let r = parse(doc).unwrap();
        assert_eq!(r.name, "a");
        assert_eq!(r.attr("x"), Some("1 & 2"));
        assert_eq!(r.child("b").unwrap().text(), "t <1> AB");
        assert_eq!(
            r.child("d").unwrap().child("e").unwrap().attr("k"),
            Some("v")
        );
        let again = parse(&r.to_document()).unwrap();
        assert_eq!(again.child("b").unwrap().text(), "t <1> AB");
        assert_eq!(again.attr("x"), Some("1 & 2"));
    }

    #[test]
    fn rejects_bad_documents() {
        for bad in [
            "",
            "<a>",
            "<a></b>",
            "<a x=1/>",
            "<a>&foo;</a>",
            "<a/><b/>",
            "<a x='1/>",
            "text",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn deep_nesting_is_limited() {
        let deep = "<a>".repeat(1000);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn truncation_never_panics() {
        let doc = "<a x=\"1\"><b>text</b><c d='e'/></a>";
        for i in 0..doc.len() {
            let _ = parse(&doc[..i]);
        }
    }
}
