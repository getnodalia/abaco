// SPDX-License-Identifier: MIT OR Apache-2.0
//! The menus Ábaco shows in the desktop's top bar (the Linux global menu), as (label, id) pairs
//! built from the command table. `-` is a separator. Every id is a command id.

use abaco_app::commands::Cmd;
use abaco_app::i18n::tr;

pub type Entry = (&'static str, &'static str);

const SEP: Entry = ("-", "");

fn items(cmds: &[Option<Cmd>]) -> Vec<Entry> {
    cmds.iter()
        .map(|c| match c {
            Some(c) => (c.translated(), c.id()),
            None => SEP,
        })
        .collect()
}

pub fn tree() -> Vec<(&'static str, Vec<Entry>)> {
    use Cmd::*;
    let some = Some;
    vec![
        (
            abaco_app::NAME,
            items(&[
                some(About),
                None,
                some(Preferences),
                some(KeyboardShortcuts),
                None,
                some(Quit),
            ]),
        ),
        (
            tr("File"),
            items(&[
                some(NewBlank),
                some(OpenSample),
                some(Open),
                None,
                some(Save),
                some(SaveAs),
            ]),
        ),
        (
            tr("Cells"),
            items(&[
                some(AddMath),
                some(AddText),
                None,
                some(EvaluateAndAdd),
                None,
                some(Duplicate),
                some(DeleteCell),
                None,
                some(MoveUp),
                some(MoveDown),
            ]),
        ),
        (
            tr("View"),
            items(&[
                some(ToggleNavigator),
                some(ToggleSidePanel),
                None,
                some(ShowUnits),
                some(ShowDocumentSettings),
                None,
                some(CommandSearch),
            ]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_id_is_a_command_and_every_command_is_listed_once() {
        let mut seen = std::collections::HashSet::new();
        for (_, entries) in tree() {
            for (label, id) in entries {
                if label != "-" {
                    assert!(Cmd::from_id(id).is_some(), "{id}");
                    assert!(seen.insert(id), "{id} appears twice");
                }
            }
        }
        for cmd in Cmd::ALL {
            assert!(seen.contains(cmd.id()), "{} is in no menu", cmd.id());
        }
    }
}
