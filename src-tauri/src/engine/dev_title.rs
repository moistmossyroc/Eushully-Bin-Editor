//! Developer scene jump (docs/engine/age-sys.md §7c). Every SYS4/SYS5 `TITLE.BIN` still
//! carries the developer menu ("デバッグメニュー": ＡＤＶパート → `0x140` jump picker over
//! `SC????.BIN`, map / battle / ADV debug runners …), placed right after the title's Quit
//! action — which always ends the process first: `call-script QUIT.BIN` (that script ends
//! in `0x1` throw-exit-request; Atelial, Madou, LaDEA, Sankai, Tenmei, Hyakusen) or an
//! inline `0x1` (Megami Zero, Himegari, VERITA). Replacing that exit with a no-op makes
//! the Quit button open the developer menu. Testing builds only (export "Developer menu").

use crate::bin_format::instruction::Instruction;
use crate::bin_format::opcodes::definition_for_opcode;

const OP_THROW_EXIT: u32 = 0x1;
const OP_RET: u32 = 0x5;
const OP_CALL_SCRIPT: u32 = 0x3;
const OP_JMP: u32 = 0x8c;
const OP_SLEEP: u32 = 0xc8;
const OP_PRESENT_FRAME: u32 = 0x20c;
/// Title of the developer menu (a `set-string` right after the Quit action).
const DEBUG_MENU_TITLE: &str = "デバッグメニュー";
/// The same title in English-patched installs (Castle Meister's fan patch: "Debug Menu").
const DEBUG_MENU_TITLE_TRANSLATED: &str = "debug menu";

fn is_debug_menu_title(text: &str) -> bool {
    text == DEBUG_MENU_TITLE || text.trim().eq_ignore_ascii_case(DEBUG_MENU_TITLE_TRANSLATED)
}

/// The exit in front of the developer menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugMenuExit {
    /// Inline `0x1` at this instruction index.
    Inline(usize),
    /// `call-script <id>` at this index, to a script that ends the process.
    QuitScript { index: usize, script: u32 },
}

/// Find the exit that guards the developer menu. `script_exits(id)` tells whether the
/// catalog script `id` ends in throw-exit-request (QUIT.BIN). Only the Quit action's own
/// instructions are searched: back from the menu title to the previous case's jmp/ret.
pub fn find_debug_menu_exit(title: &[Instruction], script_exits: impl Fn(u32) -> bool) -> Option<DebugMenuExit> {
    let menu = title
        .iter()
        .position(|i| i.arguments.iter().any(|a| a.decoded_string.as_deref().is_some_and(is_debug_menu_title)))?;
    for index in (0..menu).rev() {
        let instruction = &title[index];
        match instruction.opcode {
            OP_JMP | OP_RET => return None,
            OP_THROW_EXIT => return Some(DebugMenuExit::Inline(index)),
            OP_CALL_SCRIPT => {
                let script = instruction.arguments.first().filter(|a| a.arg_type == 0)?.raw_data;
                if script_exits(script) {
                    return Some(DebugMenuExit::QuitScript { index, script });
                }
            }
            _ => {}
        }
    }
    None
}

fn retarget(instruction: &mut Instruction, opcode: u32) {
    instruction.opcode = opcode;
    instruction.label = definition_for_opcode(opcode).map(|d| d.label.to_string()).unwrap_or_default();
}

/// Replace the exit with a same-size no-op: inline `0x1` → `present-frame` (no operands),
/// `call-script QUIT` → `sleep 0` (one immediate). Returns what was replaced.
pub fn unlock_debug_menu(title: &mut [Instruction], script_exits: impl Fn(u32) -> bool) -> Option<DebugMenuExit> {
    let exit = find_debug_menu_exit(title, script_exits)?;
    match exit {
        DebugMenuExit::Inline(index) => retarget(&mut title[index], OP_PRESENT_FRAME),
        DebugMenuExit::QuitScript { index, .. } => {
            retarget(&mut title[index], OP_SLEEP);
            title[index].arguments[0].raw_data = 0;
        }
    }
    Some(exit)
}

/// Whether a script ends the process (its last instruction is throw-exit-request).
pub fn ends_with_exit(script: &[Instruction]) -> bool {
    script.last().is_some_and(|i| i.opcode == OP_THROW_EXIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin_format::instruction::Argument;

    fn op(opcode: u32, values: &[u32]) -> Instruction {
        Instruction {
            opcode,
            label: definition_for_opcode(opcode).map(|d| d.label.to_string()).unwrap_or_default(),
            argument_count: values.len() as u32,
            arguments: values
                .iter()
                .map(|&v| Argument { arg_type: 0, raw_data: v, decoded_string: None, original_encoded_bytes: None, data_array: None })
                .collect(),
            offset: 0,
        }
    }

    fn menu_title() -> Instruction {
        let mut set = op(0x192, &[0, 0]);
        set.arguments[1].decoded_string = Some(DEBUG_MENU_TITLE.to_string());
        set
    }

    /// Atelial TITLE.BIN 136–148: CONFIG case, then the Quit case + the developer menu.
    fn atelial_title() -> Vec<Instruction> {
        vec![
            op(OP_CALL_SCRIPT, &[21230]), // CONFIG.BIN (returns)
            op(0x55, &[53, 2]),
            op(OP_JMP, &[1415]),
            op(0xb4, &[38, 1]),
            op(0xb5, &[1]),
            op(OP_CALL_SCRIPT, &[21343]), // QUIT.BIN (ends in 0x1)
            menu_title(),
        ]
    }

    #[test]
    fn quit_script_becomes_sleep() {
        let mut title = atelial_title();
        let exit = unlock_debug_menu(&mut title, |id| id == 21343);
        assert_eq!(exit, Some(DebugMenuExit::QuitScript { index: 5, script: 21343 }));
        assert_eq!((title[5].opcode, title[5].label.as_str(), title[5].arguments[0].raw_data), (OP_SLEEP, "sleep", 0));
        assert_eq!(title[5].arguments.len(), 1); // same size: offsets unchanged
        assert_eq!(title[0].opcode, OP_CALL_SCRIPT); // CONFIG untouched
    }

    #[test]
    fn inline_exit_becomes_present_frame() {
        // VERITA / Himegari: play-sound-effect, 0xb5, sleep 1000, 0x1, then the menu.
        let mut title = vec![op(OP_RET, &[]), op(OP_JMP, &[2916]), op(0xb4, &[20844, 1]), op(0xb5, &[1]), op(OP_SLEEP, &[1000]), op(OP_THROW_EXIT, &[]), menu_title()];
        assert_eq!(unlock_debug_menu(&mut title, |_| false), Some(DebugMenuExit::Inline(5)));
        assert_eq!((title[5].opcode, title[5].arguments.len()), (OP_PRESENT_FRAME, 0));
    }

    #[test]
    fn translated_menu_title_found() {
        // Castle Meister English patch: "Debug Menu" after `call-script 55` (QUIT).
        let mut title = atelial_title();
        title[6].arguments[1].decoded_string = Some("Debug Menu".to_string());
        assert_eq!(find_debug_menu_exit(&title, |id| id == 21343), Some(DebugMenuExit::QuitScript { index: 5, script: 21343 }));
    }

    #[test]
    fn nothing_outside_the_quit_case() {
        // A returning script before the menu, and the previous case's call behind its jmp.
        let mut title = atelial_title();
        assert_eq!(find_debug_menu_exit(&title, |id| id == 21230), None);
        title.retain(|i| i.opcode != 0x192);
        assert_eq!(find_debug_menu_exit(&title, |_| true), None); // no menu: no change
    }
}
