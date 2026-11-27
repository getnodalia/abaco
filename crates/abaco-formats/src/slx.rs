// SPDX-License-Identifier: MIT OR Apache-2.0
//! Simulink's packaged model format (`.slx`): a ZIP archive whose `simulink/blockdiagram.xml`
//! describes the model and whose `simulink/systems/*.xml` hold the diagrams (older files keep
//! the diagram inline). Lines refer to blocks by `SID`; this module maps them to names.

use crate::error::{corrupt, limit, unsupported, Result};
use crate::simulink::{SimBlock, SimLine, SimModel, SimSystem, MAX_BLOCKS, MAX_DEPTH};
use crate::xml::{self, Element};
use crate::zip::{Archive, Writer};

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

fn p_value(e: &Element) -> String {
    e.text().trim().to_string()
}

fn collect_config(e: &Element, out: &mut Vec<(String, String)>, depth: usize) {
    if depth > 16 {
        return;
    }
    for c in e.elements() {
        match c.name.as_str() {
            "System" | "Block" => {}
            "P" => {
                if let Some(n) = c.attr("Name") {
                    if SOLVER_KEYS.contains(&n) && !out.iter().any(|(k, _)| k == n) {
                        out.push((n.to_string(), p_value(c)));
                    }
                }
            }
            _ => collect_config(c, out, depth + 1),
        }
    }
}

fn parse_endpoint(s: &str) -> Option<(String, usize)> {
    let (sid, rest) = s.split_once('#')?;
    let port = rest
        .split_once(':')
        .and_then(|(_, n)| n.trim().parse::<usize>().ok())
        .unwrap_or(0);
    Some((sid.trim().to_string(), port))
}

fn line_dsts(e: &Element, out: &mut Vec<String>, depth: usize) -> Result<()> {
    if depth > 64 {
        return limit("branches are nested too deeply");
    }
    for c in e.elements() {
        if c.name == "P" && c.attr("Name") == Some("Dst") {
            out.push(p_value(c));
        } else if c.name == "Branch" {
            line_dsts(c, out, depth + 1)?;
        }
    }
    Ok(())
}

struct Ctx<'a> {
    zip: &'a Archive<'a>,
    blocks: usize,
}

fn load_system(ctx: &mut Ctx, e: &Element, depth: usize) -> Result<SimSystem> {
    if depth > MAX_DEPTH {
        return limit("subsystems are nested too deeply");
    }
    let owned;
    let e = match e.attr("Ref") {
        Some(r) => {
            if r.contains("..") || r.contains('/') || r.contains('\\') {
                return corrupt("a system reference is not a plain name");
            }
            owned = xml::parse_bytes(&ctx.zip.read(&format!("simulink/systems/{r}.xml"))?)?;
            &owned
        }
        None => e,
    };
    let mut sys = SimSystem::default();
    let mut sids: Vec<(String, String)> = Vec::new();
    for b in e.named("Block") {
        ctx.blocks += 1;
        if ctx.blocks > MAX_BLOCKS {
            return limit("the model has too many blocks");
        }
        let name = b.attr("Name").unwrap_or("").to_string();
        if let Some(sid) = b.attr("SID") {
            sids.push((sid.to_string(), name.clone()));
        }
        let mut blk = SimBlock {
            block_type: b.attr("BlockType").unwrap_or("").to_string(),
            name,
            ..SimBlock::default()
        };
        for c in b.elements() {
            match c.name.as_str() {
                "P" => {
                    if let Some(n) = c.attr("Name") {
                        if blk.param(n).is_none() {
                            blk.params.push((n.to_string(), p_value(c)));
                        }
                    }
                }
                "InstanceData" => {
                    for p in c.named("P") {
                        if let Some(n) = p.attr("Name") {
                            if blk.param(n).is_none() {
                                blk.params.push((n.to_string(), p_value(p)));
                            }
                        }
                    }
                }
                "System" => {
                    let mut inner = load_system(ctx, c, depth + 1)?;
                    inner.name = blk.name.clone();
                    blk.system = Some(inner);
                }
                _ => {}
            }
        }
        sys.blocks.push(blk);
    }
    let name_of = |sid: &str| sids.iter().find(|(s, _)| s == sid).map(|(_, n)| n.clone());
    for l in e.named("Line") {
        let src = l
            .named("P")
            .find(|p| p.attr("Name") == Some("Src"))
            .map(p_value)
            .and_then(|s| parse_endpoint(&s));
        let Some((src_sid, src_port)) = src else {
            continue;
        };
        let Some(src_name) = name_of(&src_sid) else {
            return corrupt(format!("a line starts at the unknown block {src_sid}"));
        };
        let mut dsts = Vec::new();
        line_dsts(l, &mut dsts, 0)?;
        for d in dsts {
            let Some((sid, port)) = parse_endpoint(&d) else {
                continue;
            };
            let Some(name) = name_of(&sid) else {
                return corrupt(format!("a line ends at the unknown block {sid}"));
            };
            sys.lines.push(SimLine {
                src: (src_name.clone(), src_port),
                dst: (name, port),
            });
        }
    }
    Ok(sys)
}

/// Reads an `.slx` file.
pub fn read(bytes: &[u8]) -> Result<SimModel> {
    let zip = Archive::open(bytes)?;
    if !zip.contains("simulink/blockdiagram.xml") {
        return corrupt("this is not a Simulink model: simulink/blockdiagram.xml is missing");
    }
    let doc = xml::parse_bytes(&zip.read("simulink/blockdiagram.xml")?)?;
    let model = if doc.name == "Model" {
        &doc
    } else {
        match doc.child("Model") {
            Some(m) => m,
            None => return corrupt("blockdiagram.xml has no Model element"),
        }
    };
    let name = model
        .named("P")
        .find(|p| p.attr("Name") == Some("Name"))
        .map(p_value)
        .unwrap_or_default();
    let mut params: Vec<(String, String)> = Vec::new();
    for p in model.named("P") {
        if let Some(n) = p.attr("Name") {
            if n != "Name" {
                params.push((n.to_string(), p_value(p)));
            }
        }
    }
    collect_config(model, &mut params, 0);
    let mut ctx = Ctx {
        zip: &zip,
        blocks: 0,
    };
    let system = match model.child("System") {
        Some(s) => load_system(&mut ctx, s, 0)?,
        None if zip.contains("simulink/systems/system_root.xml") => {
            let root = xml::parse_bytes(&zip.read("simulink/systems/system_root.xml")?)?;
            load_system(&mut ctx, &root, 0)?
        }
        None => return corrupt("the model has no System"),
    };
    Ok(SimModel {
        name: name.clone(),
        params,
        system: SimSystem { name, ..system },
    })
}

struct Out {
    zip: Writer,
    sid: usize,
    systems: Vec<String>,
}

fn p(name: &str, v: &str) -> Element {
    Element::with_text("P", v).with_attr("Name", name)
}

fn build_system(o: &mut Out, sys: &SimSystem, depth: usize) -> Result<Element> {
    if depth > MAX_DEPTH {
        return limit("subsystems are nested too deeply");
    }
    let mut e = Element::new("System");
    let mut sids: Vec<(String, usize)> = Vec::new();
    for b in &sys.blocks {
        o.sid += 1;
        let sid = o.sid;
        sids.push((b.name.clone(), sid));
        let mut be = Element::new("Block")
            .with_attr("BlockType", &b.block_type)
            .with_attr("Name", &b.name)
            .with_attr("SID", &sid.to_string());
        for (k, v) in &b.params {
            be.push(p(k, v));
        }
        if let Some(inner) = &b.system {
            let file = format!("system_{sid}");
            let ie = build_system(o, inner, depth + 1)?;
            o.zip.add(
                &format!("simulink/systems/{file}.xml"),
                ie.to_document().as_bytes(),
            )?;
            o.systems.push(file.clone());
            be.push(Element::new("System").with_attr("Ref", &file));
        }
        e.push(be);
    }
    let sid_of = |n: &str| sids.iter().find(|(m, _)| m == n).map(|(_, s)| *s);
    let mut srcs: Vec<&(String, usize)> = Vec::new();
    for l in &sys.lines {
        if !srcs.contains(&&l.src) {
            srcs.push(&l.src);
        }
    }
    for src in srcs {
        let Some(ssid) = sid_of(&src.0) else {
            return corrupt(format!("a line starts at the unknown block `{}`", src.0));
        };
        let mut le = Element::new("Line");
        le.push(p("Src", &format!("{ssid}#out:{}", src.1)));
        let dsts: Vec<&(String, usize)> = sys
            .lines
            .iter()
            .filter(|l| &l.src == src)
            .map(|l| &l.dst)
            .collect();
        let mut ends = Vec::new();
        for d in dsts {
            let Some(dsid) = sid_of(&d.0) else {
                return corrupt(format!("a line ends at the unknown block `{}`", d.0));
            };
            ends.push(format!("{dsid}#in:{}", d.1));
        }
        if let [one] = ends.as_slice() {
            le.push(p("Dst", one));
        } else {
            for d in ends {
                let mut br = Element::new("Branch");
                br.push(p("Dst", &d));
                le.push(br);
            }
        }
        e.push(le);
    }
    Ok(e)
}

/// Writes a model as an `.slx` archive.
pub fn write(m: &SimModel) -> Result<Vec<u8>> {
    if m.name.is_empty() {
        return unsupported("a model needs a name to be saved as .slx");
    }
    let mut o = Out {
        zip: Writer::new(),
        sid: 0,
        systems: Vec::new(),
    };
    let root = build_system(&mut o, &m.system, 0)?;
    o.zip.add(
        "simulink/systems/system_root.xml",
        root.to_document().as_bytes(),
    )?;
    let mut model = Element::new("Model");
    model.push(p("Name", &m.name));
    for (k, v) in &m.params {
        model.push(p(k, v));
    }
    model.push(Element::new("System").with_attr("Ref", "system_root"));
    let mut info = Element::new("ModelInformation").with_attr("Version", "1.0");
    info.push(model);
    o.zip
        .add("simulink/blockdiagram.xml", info.to_document().as_bytes())?;
    let types = r#"<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/vnd.mathworks.simulink.blockdiagram+xml"/>
  <Override PartName="/metadata/coreProperties.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
</Types>
"#;
    let rels = r#"<?xml version="1.0" encoding="utf-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.mathworks.com/simulink/2010/relationships/officeDocument" Target="simulink/blockdiagram.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="metadata/coreProperties.xml"/>
</Relationships>
"#;
    let mut core = Element::new("cp:coreProperties")
        .with_attr(
            "xmlns:cp",
            "http://schemas.openxmlformats.org/package/2006/metadata/core-properties",
        )
        .with_attr("xmlns:dc", "http://purl.org/dc/elements/1.1/");
    core.push(Element::with_text("dc:title", &m.name));
    core.push(Element::with_text("dc:creator", "Abaco"));
    o.zip.add("[Content_Types].xml", types.as_bytes())?;
    o.zip.add("_rels/.rels", rels.as_bytes())?;
    o.zip
        .add("metadata/coreProperties.xml", core.to_document().as_bytes())?;
    Ok(o.zip.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SimModel {
        let mut sub = SimSystem {
            name: "Sub".into(),
            ..SimSystem::default()
        };
        sub.blocks.push(SimBlock {
            block_type: "Inport".into(),
            name: "In1".into(),
            ..SimBlock::default()
        });
        let mut sys = SimSystem {
            name: "demo".into(),
            ..SimSystem::default()
        };
        sys.blocks.push(SimBlock {
            block_type: "Step".into(),
            name: "Step".into(),
            params: vec![("Time".into(), "1 & 2".into())],
            system: None,
        });
        sys.blocks.push(SimBlock {
            block_type: "SubSystem".into(),
            name: "Sub".into(),
            params: vec![],
            system: Some(sub),
        });
        sys.blocks.push(SimBlock {
            block_type: "Scope".into(),
            name: "S".into(),
            ..SimBlock::default()
        });
        for d in [("Sub", 1), ("S", 1)] {
            sys.lines.push(SimLine {
                src: ("Step".into(), 1),
                dst: (d.0.into(), d.1),
            });
        }
        SimModel {
            name: "demo".into(),
            params: vec![("StopTime".into(), "5".into())],
            system: sys,
        }
    }

    #[test]
    fn round_trip() {
        let m = sample();
        let bytes = write(&m).unwrap();
        let back = read(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn damage_is_an_error() {
        let bytes = write(&sample()).unwrap();
        for cut in (0..bytes.len()).step_by(7) {
            let _ = read(&bytes[..cut]);
        }
        assert!(read(b"not a zip").is_err());
    }
}
