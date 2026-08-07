#![cfg(unix)]

use std::io::{self, Cursor, Write};

#[path = "setup/claude.rs"]
mod claude;
#[path = "setup/codex.rs"]
mod codex;
mod common;
#[path = "setup/opencode.rs"]
mod opencode;
#[path = "setup/safety.rs"]
mod safety;
#[path = "setup/support.rs"]
mod support;

#[test]
fn legacy_public_setup_entry_points_remain_claude_compatible() {
    let _: fn(tapas::setup::Action, bool, &mut dyn Write, &mut dyn Write) -> io::Result<i32> =
        tapas::setup::configure;

    let mut input = Cursor::new(br#"{"tool_input":{"command":"git status"}}"#);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = tapas::setup::hook_eval(&mut input, &mut stdout, &mut stderr, false).unwrap();

    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    assert!(
        stdout
            .windows(b"\"updatedInput\"".len())
            .any(|part| part == b"\"updatedInput\"")
    );
    assert!(
        !stdout
            .windows(b"permissionDecision".len())
            .any(|part| part == b"permissionDecision")
    );
}
