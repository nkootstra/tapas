use std::ffi::OsStr;
use std::path::Path;

// The generated compatibility catalogs are included in a separate file so
// their contents remain directly reproducible from the pinned U1 inventory.
include!("catalog.generated.rs");

pub fn command_basename(command: &OsStr) -> Option<&OsStr> {
    Path::new(command).file_name()
}

pub fn should_auto_wrap(command: &OsStr) -> bool {
    let Some(basename) = command_basename(command) else {
        return false;
    };
    AUTO_WRAP_COMMANDS
        .iter()
        .any(|candidate| basename == OsStr::new(candidate))
}
