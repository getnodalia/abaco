// SPDX-License-Identifier: MIT OR Apache-2.0
//! A developer tool: `matcheck read FILE` prints what a MAT-file holds; `matcheck write DIR`
//! writes a sample in every level, for checking against other readers.

use abaco_formats::mat::{self, MatFile, MatValue, MatVersion, NumClass};

fn sample() -> Vec<(String, MatValue)> {
    vec![
        (
            "a".into(),
            MatValue::Numeric {
                class: NumClass::Double,
                dims: vec![2, 3],
                re: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                im: None,
            },
        ),
        ("s".into(), MatValue::text("hello")),
        (
            "z".into(),
            MatValue::Numeric {
                class: NumClass::Double,
                dims: vec![1, 2],
                re: vec![1.0, 2.0],
                im: Some(vec![-1.0, 0.5]),
            },
        ),
        (
            "i".into(),
            MatValue::Numeric {
                class: NumClass::Int16,
                dims: vec![1, 3],
                re: vec![-1.0, 0.0, 300.0],
                im: None,
            },
        ),
        (
            "b".into(),
            MatValue::Logical {
                dims: vec![2, 2],
                data: vec![true, false, false, true],
            },
        ),
        (
            "c".into(),
            MatValue::Cell {
                dims: vec![1, 2],
                items: vec![MatValue::scalar(7.0), MatValue::text("x")],
            },
        ),
        (
            "st".into(),
            MatValue::Struct {
                dims: vec![1, 1],
                fields: vec!["u".into(), "v".into()],
                values: vec![MatValue::scalar(1.0), MatValue::text("hi")],
            },
        ),
        (
            "nd".into(),
            MatValue::Numeric {
                class: NumClass::Single,
                dims: vec![2, 2, 2],
                re: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
                im: None,
            },
        ),
        (
            "sp".into(),
            MatValue::Sparse {
                rows: 3,
                cols: 3,
                row_idx: vec![0, 2, 1],
                col_ptr: vec![0, 2, 2, 3],
                re: vec![1.0, 2.0, 3.0],
                im: None,
                logical: false,
            },
        ),
    ]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match (args.get(1).map(String::as_str), args.get(2)) {
        (Some("read"), Some(p)) => match std::fs::read(p).map(|b| mat::read(&b)) {
            Ok(Ok(f)) => {
                println!("{:?} {:?}", f.version, f.description);
                for (n, v) in &f.vars {
                    println!("{n} = {v:?}");
                }
            }
            Ok(Err(e)) => println!("error: {e}"),
            Err(e) => println!("io: {e}"),
        },
        (Some("write"), Some(d)) => {
            for (v, name, keep) in [
                (MatVersion::V4, "v4", 7usize),
                (MatVersion::V6, "v6", 9),
                (MatVersion::V7, "v7", 9),
                (MatVersion::V73, "v73", 12),
            ] {
                let vars: Vec<_> = sample().into_iter().take(keep).collect();
                let vars: Vec<_> = if v == MatVersion::V4 {
                    vars.into_iter()
                        .filter(|(n, _)| !matches!(n.as_str(), "c" | "st" | "nd"))
                        .collect()
                } else {
                    vars
                };
                let f = MatFile {
                    version: v,
                    description: String::new(),
                    vars,
                };
                match mat::write(&f) {
                    Ok(b) => {
                        let _ = std::fs::write(format!("{d}/{name}.mat"), b);
                    }
                    Err(e) => println!("{name}: {e}"),
                }
            }
        }
        _ => println!("usage: matcheck read FILE | matcheck write DIR"),
    }
}
