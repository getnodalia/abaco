// SPDX-License-Identifier: MIT OR Apache-2.0
//! The block-diagram workspace: the diagram being edited and what it needs to be tested without
//! a display ([`session`]), and Ábaco Blocks on a worker thread ([`engine`]).

pub mod engine;
pub mod session;

/// A parameter's name as a label: `sample_time` becomes `Sample time`. The result is looked up
/// in the Spanish table, so every name a block has needs a row there (a test checks).
pub fn field_label(key: &str) -> String {
    let text = key.replace('_', " ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::field_label;
    use crate::i18n::translate_known;

    #[test]
    fn labels_are_humanised() {
        assert_eq!(field_label("sample_time"), "Sample time");
        assert_eq!(field_label("kp"), "Kp");
        assert_eq!(field_label(""), "");
    }

    #[test]
    fn every_block_category_and_parameter_has_spanish() {
        let mut missing: Vec<String> = Vec::new();
        for category in abaco_blocks::CATEGORIES {
            if translate_known(category).is_none() {
                missing.push(category.to_owned());
            }
        }
        for entry in abaco_blocks::palette() {
            if translate_known(entry.name).is_none() {
                missing.push(entry.name.to_owned());
            }
            if let Ok(serde_json::Value::Object(map)) = serde_json::to_value(&entry.kind) {
                for key in map.keys().filter(|k| *k != "type") {
                    let label = field_label(key);
                    if translate_known(&label).is_none() {
                        missing.push(label);
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "without Spanish: {missing:?}");
    }
}
