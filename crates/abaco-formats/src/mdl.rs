// SPDX-License-Identifier: MIT OR Apache-2.0
//! Simulink's text model format (`.mdl`): a tree of `Name { … }` sections holding `Key value`
//! lines, where a value is a quoted string (long ones continued by a following quoted string), a
//! bracketed list or a bare word.

use crate::error::{corrupt, limit, Result};
use crate::simulink::{SimBlock, SimLine, SimModel, SimSystem, MAX_BLOCKS, MAX_DEPTH};

const MAX_TREE_DEPTH: usize = 4 * MAX_DEPTH + 16;
const MAX_NODES: usize = 2_000_000;

#[derive(Debug, Default)]
struct Node {
    kind: String,
    props: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Node {
    fn prop(&self, k: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    nodes: usize,
}

impl Parser<'_> {
    fn line(&self) -> usize {
        1 + self.s.iter().take(self.i).filter(|&&b| b == b'\n').count()
    }

    fn err<T>(&self, msg: &str) -> Result<T> {
        corrupt(format!("invalid model file at line {}: {msg}", self.line()))
    }

    fn skip(&mut self) {
        loop {
            match self.s.get(self.i) {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.i += 1,
                Some(b'#') => {
                    while self.s.get(self.i).is_some_and(|&b| b != b'\n') {
                        self.i += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn quoted(&mut self) -> Result<String> {
        // At an opening quote.
        let mut out: Vec<u8> = Vec::new();
        self.i += 1;
        loop {
            match self.s.get(self.i) {
                None => return self.err("a string is not closed"),
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.s.get(self.i) {
                        Some(b'n') => out.push(b'\n'),
                        Some(b't') => out.push(b'\t'),
                        Some(&c) => out.push(c),
                        None => return self.err("a string is not closed"),
                    }
                    self.i += 1;
                }
                Some(&c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn value(&mut self) -> Result<String> {
        match self.s.get(self.i) {
            Some(b'"') => {
                let mut v = self.quoted()?;
                loop {
                    let save = self.i;
                    self.skip();
                    if self.s.get(self.i) == Some(&b'"') {
                        v.push_str(&self.quoted()?);
                    } else {
                        self.i = save;
                        break;
                    }
                }
                Ok(v)
            }
            Some(b'[') => {
                let start = self.i;
                let mut depth = 0usize;
                while let Some(&b) = self.s.get(self.i) {
                    match b {
                        b'[' => depth += 1,
                        b']' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                self.i += 1;
                                break;
                            }
                        }
                        b'"' => {
                            self.quoted()?;
                            continue;
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
                Ok(self.text.get(start..self.i).unwrap_or("").to_string())
            }
            Some(_) => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|&b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'}'))
                {
                    self.i += 1;
                }
                Ok(self.text.get(start..self.i).unwrap_or("").to_string())
            }
            None => self.err("a value is missing"),
        }
    }

    fn body(&mut self, node: &mut Node, depth: usize) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            return limit("sections are nested too deeply");
        }
        loop {
            self.skip();
            match self.s.get(self.i) {
                None => {
                    return if depth == 0 {
                        Ok(())
                    } else {
                        self.err("a section is not closed")
                    };
                }
                Some(b'}') => {
                    if depth == 0 {
                        return self.err("an unmatched `}`");
                    }
                    self.i += 1;
                    return Ok(());
                }
                Some(_) => {}
            }
            let start = self.i;
            while self
                .s
                .get(self.i)
                .is_some_and(|&b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'"'))
            {
                self.i += 1;
            }
            let word = self.text.get(start..self.i).unwrap_or("").to_string();
            if word.is_empty() {
                return self.err("a name was expected");
            }
            self.skip();
            self.nodes += 1;
            if self.nodes > MAX_NODES {
                return limit("the model file has too many entries");
            }
            if self.s.get(self.i) == Some(&b'{') {
                self.i += 1;
                let mut child = Node {
                    kind: word,
                    ..Node::default()
                };
                self.body(&mut child, depth + 1)?;
                node.children.push(child);
            } else {
                let v = self.value()?;
                node.props.push((word, v));
            }
        }
    }
}

fn parse_tree(text: &str) -> Result<Node> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut root = Node::default();
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
        nodes: 0,
    };
    p.body(&mut root, 0)?;
    Ok(root)
}

const SOLVER_KEYS: &[&str] = &[
    "StartTime",
    "StopTime",
    "SolverName",
    "Solver",
    "SolverType",
    "FixedStep",
    "MaxStep",
    "InitialStep",
    "RelTol",
    "AbsTol",
];

fn collect_config(n: &Node, out: &mut Vec<(String, String)>, depth: usize) {
    if depth > 16 {
        return;
    }
    for c in &n.children {
        if c.kind == "System" {
            continue;
        }
        if c.prop("ClassName").is_some_and(|k| k.contains("Solver")) {
            for (k, v) in &c.props {
                if SOLVER_KEYS.contains(&k.as_str()) && !out.iter().any(|(n, _)| n == k) {
                    out.push((k.clone(), v.clone()));
                }
            }
        }
        collect_config(c, out, depth + 1);
    }
}

fn parse_port(s: &str) -> usize {
    s.trim().parse().unwrap_or(0)
}

fn branch_dsts(n: &Node, out: &mut Vec<(String, usize)>, depth: usize) -> Result<()> {
    if depth > MAX_TREE_DEPTH {
        return limit("branches are nested too deeply");
    }
    if let Some(b) = n.prop("DstBlock") {
        out.push((b.to_string(), parse_port(n.prop("DstPort").unwrap_or("1"))));
    }
    for c in n.children.iter().filter(|c| c.kind == "Branch") {
        branch_dsts(c, out, depth + 1)?;
    }
    Ok(())
}

fn system(n: &Node, depth: usize, count: &mut usize) -> Result<SimSystem> {
    if depth > MAX_DEPTH {
        return limit("subsystems are nested too deeply");
    }
    let mut sys = SimSystem {
        name: n.prop("Name").unwrap_or("").to_string(),
        ..SimSystem::default()
    };
    for c in &n.children {
        match c.kind.as_str() {
            "Block" => {
                *count += 1;
                if *count > MAX_BLOCKS {
                    return limit("the model has too many blocks");
                }
                let mut b = SimBlock {
                    block_type: c.prop("BlockType").unwrap_or("").to_string(),
                    name: c.prop("Name").unwrap_or("").to_string(),
                    ..SimBlock::default()
                };
                for (k, v) in &c.props {
                    if k != "BlockType" && k != "Name" && b.param(k).is_none() {
                        b.params.push((k.clone(), v.clone()));
                    }
                }
                if let Some(s) = c.children.iter().find(|s| s.kind == "System") {
                    b.system = Some(system(s, depth + 1, count)?);
                }
                sys.blocks.push(b);
            }
            "Line" => {
                let Some(src) = c.prop("SrcBlock") else {
                    continue;
                };
                let from = (
                    src.to_string(),
                    parse_port(c.prop("SrcPort").unwrap_or("1")),
                );
                let mut dsts = Vec::new();
                branch_dsts(c, &mut dsts, 0)?;
                for d in dsts {
                    sys.lines.push(SimLine {
                        src: from.clone(),
                        dst: d,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(sys)
}

/// Reads a `.mdl` file.
pub fn read(text: &str) -> Result<SimModel> {
    let root = parse_tree(text)?;
    let Some(model) = root
        .children
        .iter()
        .find(|c| c.kind == "Model" || c.kind == "Library")
    else {
        return corrupt("this is not a Simulink model: there is no `Model` section");
    };
    let name = model.prop("Name").unwrap_or("").to_string();
    let mut params: Vec<(String, String)> = model
        .props
        .iter()
        .filter(|(k, _)| k != "Name")
        .cloned()
        .collect();
    collect_config(model, &mut params, 0);
    let Some(sys) = model.children.iter().find(|c| c.kind == "System") else {
        return corrupt("the model has no `System` section");
    };
    let mut count = 0;
    Ok(SimModel {
        name,
        params,
        system: system(sys, 0, &mut count)?,
    })
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('"');
}

fn is_bare(key: &str, value: &str) -> bool {
    matches!(key, "Position" | "Points" | "Location" | "Ports")
        && value.starts_with('[')
        && !value.contains('"')
}

fn put(out: &mut String, pad: &str, k: &str, v: &str) {
    out.push_str(pad);
    out.push_str(k);
    out.push_str("\t\t");
    if is_bare(k, v) {
        out.push_str(v);
    } else {
        quote(v, out);
    }
    out.push('\n');
}

fn write_system(sys: &SimSystem, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let inner = "  ".repeat(depth + 1);
    out.push_str(&format!("{pad}System {{\n"));
    put(out, &inner, "Name", &sys.name);
    for b in &sys.blocks {
        out.push_str(&format!("{inner}Block {{\n"));
        let deep = "  ".repeat(depth + 2);
        out.push_str(&format!("{deep}BlockType\t\t{}\n", b.block_type));
        put(out, &deep, "Name", &b.name);
        for (k, v) in &b.params {
            put(out, &deep, k, v);
        }
        if let Some(s) = &b.system {
            write_system(s, depth + 2, out);
        }
        out.push_str(&format!("{inner}}}\n"));
    }
    // One Line per source; several destinations become branches.
    let mut srcs: Vec<&(String, usize)> = Vec::new();
    for l in &sys.lines {
        if !srcs.contains(&&l.src) {
            srcs.push(&l.src);
        }
    }
    for src in srcs {
        let dsts: Vec<&(String, usize)> = sys
            .lines
            .iter()
            .filter(|l| &l.src == src)
            .map(|l| &l.dst)
            .collect();
        out.push_str(&format!("{inner}Line {{\n"));
        let deep = "  ".repeat(depth + 2);
        put(out, &deep, "SrcBlock", &src.0);
        out.push_str(&format!("{deep}SrcPort\t\t{}\n", src.1));
        if let [d] = dsts.as_slice() {
            put(out, &deep, "DstBlock", &d.0);
            out.push_str(&format!("{deep}DstPort\t\t{}\n", d.1));
        } else {
            for d in dsts {
                out.push_str(&format!("{deep}Branch {{\n"));
                let br = "  ".repeat(depth + 3);
                put(out, &br, "DstBlock", &d.0);
                out.push_str(&format!("{br}DstPort\t\t{}\n", d.1));
                out.push_str(&format!("{deep}}}\n"));
            }
        }
        out.push_str(&format!("{inner}}}\n"));
    }
    out.push_str(&format!("{pad}}}\n"));
}

/// Writes a model as `.mdl` text.
pub fn write(m: &SimModel) -> String {
    let mut out = String::from("Model {\n");
    put(&mut out, "  ", "Name", &m.name);
    for (k, v) in &m.params {
        put(&mut out, "  ", k, v);
    }
    write_system(&m.system, 1, &mut out);
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
Model {
  Name		  "demo"
  Version		  8.0
  StopTime "5"
  SolverName "ode45"
  System {
    Name "demo"
    Location [0, 0, 100, 100]
    Block {
      BlockType Step
      Name "Step"
      Position [10, 10, 40, 40]
      Time "1"
    }
    Block {
      BlockType Gain
      Name "G"
      Gain "2"
      Description "a long "
        "description"
    }
    Block {
      BlockType Scope
      Name "S"
    }
    Block {
      BlockType SubSystem
      Name "Sub"
      System {
        Name "Sub"
        Block { BlockType Inport Name "In1" }
      }
    }
    Line {
      SrcBlock "Step"
      SrcPort 1
      DstBlock "G"
      DstPort 1
    }
    Line {
      SrcBlock "G"
      SrcPort 1
      Points [10, 0]
      Branch { DstBlock "S" DstPort 1 }
      Branch { Points [0, 5] DstBlock "Sub" DstPort 1 }
    }
  }
}
"#;

    #[test]
    fn reads_a_model() {
        let m = read(SAMPLE).unwrap();
        assert_eq!(m.name, "demo");
        assert_eq!(m.param("StopTime"), Some("5"));
        assert_eq!(m.system.blocks.len(), 4);
        assert_eq!(m.system.blocks[1].param("Gain"), Some("2"));
        assert_eq!(
            m.system.blocks[1].param("Description"),
            Some("a long description")
        );
        assert_eq!(m.system.blocks[3].system.as_ref().unwrap().blocks.len(), 1);
        assert_eq!(m.system.lines.len(), 3);
        assert_eq!(m.system.lines[2].dst, ("Sub".to_string(), 1));
    }

    #[test]
    fn writes_what_it_reads() {
        let m = read(SAMPLE).unwrap();
        let text = write(&m);
        let again = read(&text).unwrap();
        assert_eq!(m, again);
    }

    #[test]
    fn hostile_text_is_an_error() {
        for bad in [
            "Model {",
            "}",
            "Model { Name }",
            "Model { Name \"x }",
            &"A {".repeat(5000),
        ] {
            assert!(read(bad).is_err(), "{bad:.20}");
        }
        for i in 0..SAMPLE.len() {
            let _ = read(&SAMPLE[..i]);
        }
    }
}
