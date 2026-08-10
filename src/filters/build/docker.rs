use std::collections::BTreeSet;

use super::find_subslice;

pub(super) fn route(command: &[u8], argv: &[&[u8]]) -> bool {
    let build = command == b"docker"
        && (argv.get(1) == Some(&b"build".as_slice())
            || argv.get(1) == Some(&b"buildx".as_slice())
                && argv.get(2) == Some(&b"build".as_slice())
            || argv.get(1) == Some(&b"compose".as_slice())
                && argv.get(2) == Some(&b"build".as_slice()))
        || command == b"docker-compose" && argv.get(1) == Some(&b"build".as_slice());
    build
}

pub(super) fn compact(input: &[u8]) -> Option<Vec<u8>> {
    let mut completed = BTreeSet::new();
    let mut image = None;
    let mut recognized = false;
    for line in input.split(|byte| *byte == b'\n') {
        if !line.starts_with(b"#") {
            if !line.is_empty() {
                return None;
            }
            continue;
        }
        let id_end = line.iter().position(|byte| byte.is_ascii_whitespace())?;
        let id = &line[..id_end];
        if line[id_end..]
            .windows(b" DONE ".len())
            .any(|part| part == b" DONE ")
            || line.ends_with(b" DONE")
        {
            completed.insert(id);
            recognized = true;
        }
        if let Some(marker) = find_subslice(line, b"naming to ") {
            let name = &line[marker + b"naming to ".len()..];
            image = Some(name.strip_suffix(b" done").unwrap_or(name));
            recognized = true;
        }
    }
    if !recognized || completed.is_empty() {
        return None;
    }
    let mut output = format!("BuildKit: {} steps completed\n", completed.len()).into_bytes();
    if let Some(image) = image {
        output.extend_from_slice(b"image: ");
        output.extend_from_slice(image);
        output.push(b'\n');
    }
    Some(output)
}
