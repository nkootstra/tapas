use super::{
    EvidenceClass, FilterError, StreamFilterOutput, command_basename, find_subslice, strip_ansi,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"aws"
                    | b"jq"
                    | b"pup"
                    | b"acli"
                    | b"gh"
                    | b"cat"
                    | b"docker"
                    | b"docker-compose"
                    | b"kubectl"
                    | b"ps"
                    | b"df"
                    | b"psql"
                    | b"systemctl"
                    | b"lsof"
                    | b"npm"
                    | b"pnpm"
                    | b"yarn"
                    | b"brew"
                    | b"bun"
            )
        })
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    if lossless || requests_query(argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }

    let command = command_basename(argv[0]);
    if matches!(command, b"pup" | b"acli") && exit_code != 0 {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    if requests_machine_output(command, argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }

    let wants_json = matches!(command, b"aws" | b"jq" | b"pup" | b"acli")
        || command == b"gh" && gh_wants_data_output(argv);
    if wants_json
        && stderr.is_empty()
        && let Some(compact) = compact_json(stdout)
    {
        return Ok(StreamFilterOutput::new(
            compact,
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ));
    }

    if command == b"pup" && matches_pup_table(stdout) {
        return Ok(StreamFilterOutput::new(
            compact_pup_table(stdout),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }

    if command == b"cat"
        && stdout.len() > 512
        && !cat_requests_exact(argv)
        && let Some(compact) = compact_cat(stdout, argv)
    {
        return Ok(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }

    if is_columnar_command(command) && matches_columnar(stdout) {
        return Ok(StreamFilterOutput::new(
            compact_columnar(stdout),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }

    Ok(StreamFilterOutput::passthrough(stdout, stderr))
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Language {
    Rust,
    Zig,
    Go,
    Python,
    TypeScript,
    JavaScript,
    Java,
    Cpp,
    Ruby,
    Data,
    Unknown,
}

fn compact_cat(input: &[u8], argv: &[&[u8]]) -> Option<Vec<u8>> {
    let language = detect_language(argv);
    if language == Language::Data || language == Language::Unknown && !looks_like_code(input) {
        return None;
    }
    Some(compact_code(input, language))
}

fn detect_language(argv: &[&[u8]]) -> Language {
    let filename = argv
        .iter()
        .copied()
        .rfind(|argument| !argument.is_empty() && argument[0] != b'-')
        .unwrap_or_default();
    let basename = filename
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(filename, |slash| &filename[slash + 1..]);
    let Some(dot) = basename.iter().rposition(|byte| *byte == b'.') else {
        return Language::Unknown;
    };
    if dot == 0 {
        return Language::Unknown;
    }
    match &basename[dot + 1..] {
        b"rs" => Language::Rust,
        b"zig" => Language::Zig,
        b"go" => Language::Go,
        b"py" | b"pyi" => Language::Python,
        b"ts" | b"tsx" | b"mts" => Language::TypeScript,
        b"js" | b"jsx" | b"mjs" => Language::JavaScript,
        b"java" | b"kt" | b"kts" => Language::Java,
        b"c" | b"h" | b"cpp" | b"cc" | b"hpp" | b"hh" => Language::Cpp,
        b"rb" => Language::Ruby,
        b"json" | b"yaml" | b"yml" | b"toml" | b"xml" | b"csv" | b"sql" | b"md" | b"markdown"
        | b"html" | b"css" | b"txt" | b"env" | b"ini" | b"cfg" | b"conf" | b"lock" | b"sum" => {
            Language::Data
        }
        _ => Language::Unknown,
    }
}

fn looks_like_code(input: &[u8]) -> bool {
    let input = &input[..input.len().min(2048)];
    [
        b"import ".as_slice(),
        b"#include",
        b"def ",
        b"fn ",
        b"func ",
        b"function ",
        b"class ",
        b"pub ",
        b"const ",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

fn compact_code(input: &[u8], language: Language) -> Vec<u8> {
    let lines = input.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0usize;
    let mut brace_depth = 0isize;
    let mut in_body = false;
    let mut body_lines = 0usize;
    let mut previous_blank = false;

    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let trimmed = trim_code_line(line);
        if trimmed.is_empty() {
            if !previous_blank && !in_body {
                output.push(b'\n');
            }
            previous_blank = true;
            continue;
        }
        previous_blank = false;

        if is_import_line(trimmed)
            || is_doc_comment(trimmed, language)
            || trimmed[0] == b'@'
            || trimmed.starts_with(b"#[")
        {
            write_code_line(&mut output, line);
            continue;
        }

        if !matches!(language, Language::Python | Language::Ruby) {
            let opens = trimmed.iter().filter(|byte| **byte == b'{').count() as isize;
            let closes = trimmed.iter().filter(|byte| **byte == b'}').count() as isize;
            if in_body {
                brace_depth += opens - closes;
                body_lines += 1;
                if brace_depth <= 0 {
                    if body_lines > 1 {
                        output.extend_from_slice(b"    // ... (");
                        output.extend_from_slice(body_lines.to_string().as_bytes());
                        output.extend_from_slice(b" lines)\n");
                    }
                    write_code_line(&mut output, line);
                    in_body = false;
                    body_lines = 0;
                }
                continue;
            }
            if is_elision_trigger(trimmed, language) {
                write_code_line(&mut output, line);
                if opens > closes {
                    brace_depth = opens - closes;
                    in_body = true;
                    body_lines = 0;
                }
                continue;
            }
        } else if is_python_signature(trimmed) {
            write_code_line(&mut output, line);
            let signature_indent = leading_spaces(line);
            let mut skipped = 0usize;
            while index < lines.len() {
                let body_line = lines[index];
                index += 1;
                let body_trimmed = trim_code_line(body_line);
                if body_trimmed.is_empty() || leading_spaces(body_line) > signature_indent {
                    skipped += 1;
                    continue;
                }
                if skipped > 0 {
                    output.extend_from_slice(b"    # ... (");
                    output.extend_from_slice(skipped.to_string().as_bytes());
                    output.extend_from_slice(b" lines)\n");
                }
                write_code_line(&mut output, body_line);
                break;
            }
            continue;
        }

        write_code_line(&mut output, line);
    }
    output
}

fn trim_code_line(mut line: &[u8]) -> &[u8] {
    while line
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        line = &line[1..];
    }
    while line
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        line = &line[..line.len() - 1];
    }
    line
}

fn write_code_line(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(line);
    output.push(b'\n');
}

fn is_import_line(line: &[u8]) -> bool {
    line.starts_with(b"import ")
        || line.starts_with(b"from ")
        || line.starts_with(b"#include")
        || line.starts_with(b"use ")
        || line.starts_with(b"require")
        || line.starts_with(b"const ") && find_subslice(line, b"require(").is_some()
        || line.starts_with(b"export ")
}

fn is_doc_comment(line: &[u8], language: Language) -> bool {
    match language {
        Language::Rust | Language::Zig => line.starts_with(b"///") || line.starts_with(b"//!"),
        Language::Go => line.starts_with(b"//"),
        Language::Python => line.starts_with(b"\"\"\"") || line.starts_with(b"'''"),
        Language::Java => line.starts_with(b"/**"),
        Language::TypeScript | Language::JavaScript | Language::Cpp => {
            line.starts_with(b"/**") || line.starts_with(b"///")
        }
        Language::Ruby => line.starts_with(b"##"),
        Language::Data | Language::Unknown => false,
    }
}

fn is_elision_trigger(line: &[u8], language: Language) -> bool {
    match language {
        Language::Rust => {
            line.starts_with(b"pub fn ")
                || line.starts_with(b"fn ")
                || line.starts_with(b"pub async fn ")
                || line.starts_with(b"async fn ")
        }
        Language::Zig => {
            line.starts_with(b"pub fn ") || line.starts_with(b"fn ") || line.starts_with(b"test ")
        }
        Language::Go => line.starts_with(b"func "),
        Language::TypeScript | Language::JavaScript => {
            line.starts_with(b"function ")
                || line.starts_with(b"async function ")
                || is_ts_arrow_function(line)
                || is_ts_method_shorthand(line)
        }
        Language::Java => {
            (line.starts_with(b"public ")
                || line.starts_with(b"private ")
                || line.starts_with(b"protected "))
                && line.contains(&b'(')
                && find_subslice(line, b"class ").is_none()
                && find_subslice(line, b"interface ").is_none()
        }
        Language::Cpp => line.contains(&b'(') && (line.ends_with(b"{") || line.ends_with(b")")),
        _ => false,
    }
}

fn is_ts_arrow_function(line: &[u8]) -> bool {
    if !line.ends_with(b"{") || line.starts_with(b"type ") {
        return false;
    }
    let Some(arrow) = find_subslice(line, b"=>") else {
        return false;
    };
    find_subslice(&line[..arrow], b" = ").is_some()
}

fn is_ts_method_shorthand(line: &[u8]) -> bool {
    if !line.ends_with(b"{") || find_subslice(line, b"=>").is_some() {
        return false;
    }
    let Some(open) = line.iter().position(|byte| *byte == b'(') else {
        return false;
    };
    if open == 0 || !line.contains(&b')') {
        return false;
    }
    let head = trim_code_line(&line[..open]);
    let mut saw_token = false;
    for token in head
        .split(|byte| *byte == b' ')
        .filter(|token| !token.is_empty())
    {
        saw_token = true;
        if matches!(
            token,
            b"if"
                | b"else"
                | b"for"
                | b"while"
                | b"switch"
                | b"catch"
                | b"do"
                | b"with"
                | b"return"
                | b"case"
                | b"function"
                | b"class"
                | b"interface"
                | b"enum"
                | b"namespace"
                | b"type"
                | b"new"
                | b"typeof"
                | b"await"
                | b"yield"
                | b"throw"
                | b"delete"
                | b"void"
                | b"in"
                | b"of"
                | b"import"
                | b"export"
        ) || !is_ident_like_token(token)
        {
            return false;
        }
    }
    saw_token
}

fn is_ident_like_token(token: &[u8]) -> bool {
    let token = token.strip_prefix(b"*").unwrap_or(token);
    !token.is_empty()
        && token
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
}

fn is_python_signature(line: &[u8]) -> bool {
    line.starts_with(b"def ") || line.starts_with(b"async def ") || line.starts_with(b"class ")
}

fn leading_spaces(line: &[u8]) -> usize {
    line.iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .map(|byte| if *byte == b'\t' { 4 } else { 1 })
        .sum()
}

fn cat_requests_exact(argv: &[&[u8]]) -> bool {
    let mut options = true;
    for argument in &argv[1..] {
        if options && *argument == b"--" {
            options = false;
        } else if options && argument.starts_with(b"-") {
            return true;
        }
    }
    false
}

fn matches_pup_table(input: &[u8]) -> bool {
    let mut saw_border = false;
    let mut saw_row = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if is_pup_border(line) {
            saw_border = true;
        } else if is_pup_pipe_row(line) {
            saw_row = true;
        }
    }
    saw_border && saw_row
}

fn compact_pup_table(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.strip_suffix(b"\r").unwrap_or(&clean);
        if line.is_empty() || is_pup_border(line) || is_pup_separator(line) {
            continue;
        }
        if !is_pup_pipe_row(line) {
            output.extend_from_slice(line);
            output.push(b'\n');
            continue;
        }

        let start = usize::from(line.first() == Some(&b'|'));
        let end = if line.len() > start && line.last() == Some(&b'|') {
            line.len() - 1
        } else {
            line.len()
        };
        let mut fields = line[start..end]
            .split(|byte| *byte == b'|')
            .map(|field| field.trim_ascii())
            .collect::<Vec<_>>();
        while fields.last().is_some_and(|field| field.is_empty()) {
            fields.pop();
        }
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                output.push(b'\t');
            }
            output.extend_from_slice(field);
        }
        if !fields.is_empty() {
            output.push(b'\n');
        }
    }
    output
}

fn is_pup_pipe_row(line: &[u8]) -> bool {
    line.len() >= 2 && line[0] == b'|' && line[1..].contains(&b'|')
}

fn is_pup_border(line: &[u8]) -> bool {
    !line.is_empty()
        && line[0] == b'+'
        && line.iter().all(|byte| matches!(byte, b'+' | b'-' | b'='))
}

fn is_pup_separator(line: &[u8]) -> bool {
    is_pup_pipe_row(line) && line.iter().all(|byte| matches!(byte, b'|' | b'-'))
}

fn matches_columnar(input: &[u8]) -> bool {
    input.windows(2).any(|window| window == b"  ")
}

fn compact_columnar(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous: Vec<&[u8]> = Vec::new();
    let mut repeated_rows = 0usize;
    let mut offset = 0usize;

    while offset < input.len() {
        let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
        let end = relative_end.map_or(input.len(), |index| offset + index);
        let line = &input[offset..end];
        let has_newline = end < input.len();

        if !line.windows(2).any(|window| window == b"  ") {
            flush_repeated_rows(&mut output, &mut repeated_rows);
            output.extend_from_slice(line);
            if has_newline {
                output.push(b'\n');
            }
            previous.clear();
        } else {
            let fields = split_columnar_fields(line);
            if !previous.is_empty() && fields == previous {
                repeated_rows += 1;
            } else {
                flush_repeated_rows(&mut output, &mut repeated_rows);
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        output.push(b' ');
                    }
                    if !previous.is_empty()
                        && previous.get(index).is_some_and(|prior| prior == field)
                        && !field.is_empty()
                    {
                        output.push(b'~');
                    } else if index + 1 == fields.len() {
                        write_truncated_last_field(&mut output, field);
                    } else {
                        output.extend_from_slice(field);
                    }
                }
                if has_newline {
                    output.push(b'\n');
                }
                previous = fields;
            }
        }
        offset = end + usize::from(has_newline);
    }
    flush_repeated_rows(&mut output, &mut repeated_rows);
    output
}

fn split_columnar_fields(line: &[u8]) -> Vec<&[u8]> {
    let mut fields = Vec::new();
    let mut index = 0usize;
    while index < line.len() && line[index] == b' ' {
        index += 1;
    }
    while index < line.len() {
        let start = index;
        while index < line.len() {
            if line[index] != b' ' {
                index += 1;
                continue;
            }
            let mut after = index;
            while after < line.len() && line[after] == b' ' {
                after += 1;
            }
            if after - index >= 2 {
                break;
            }
            index = after;
        }
        if index > start {
            fields.push(&line[start..index]);
        }
        while index < line.len() && line[index] == b' ' {
            index += 1;
        }
    }
    fields
}

fn flush_repeated_rows(output: &mut Vec<u8>, repeated_rows: &mut usize) {
    if *repeated_rows > 0 {
        output.extend_from_slice(b"~ x");
        output.extend_from_slice(repeated_rows.to_string().as_bytes());
        output.push(b'\n');
        *repeated_rows = 0;
    }
}

fn write_truncated_last_field(output: &mut Vec<u8>, field: &[u8]) {
    if field.first() == Some(&b'/') {
        if let Some(slash) = field.iter().rposition(|byte| *byte == b'/')
            && slash + 1 < field.len()
        {
            output.extend_from_slice(&field[slash + 1..]);
            return;
        }
    } else if let Some(space) = field.windows(2).position(|window| window == b" /") {
        output.extend_from_slice(&field[..=space]);
        let path = &field[space + 1..];
        if let Some(slash) = path.iter().rposition(|byte| *byte == b'/')
            && slash + 1 < path.len()
        {
            output.extend_from_slice(&path[slash + 1..]);
            return;
        }
        output.extend_from_slice(path);
        return;
    }
    output.extend_from_slice(field);
}

fn is_columnar_command(command: &[u8]) -> bool {
    matches!(
        command,
        b"docker"
            | b"docker-compose"
            | b"kubectl"
            | b"gh"
            | b"ps"
            | b"df"
            | b"psql"
            | b"systemctl"
            | b"lsof"
            | b"npm"
            | b"pnpm"
            | b"yarn"
            | b"brew"
            | b"bun"
    )
}

fn compact_json(input: &[u8]) -> Option<Vec<u8>> {
    let input = trim_bom_and_space(input);
    if !is_single_top_level_container(input) {
        return None;
    }

    let mut output = Vec::with_capacity(input.len() + 1);
    let mut in_string = false;
    let mut escaped = false;
    for &byte in input {
        if in_string {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
            output.push(byte);
        } else if !byte.is_ascii_whitespace() {
            output.push(byte);
        }
    }
    output.push(b'\n');
    Some(output)
}

fn is_single_top_level_container(input: &[u8]) -> bool {
    let Some((&root, _)) = input.split_first() else {
        return false;
    };
    let expected_root = match root {
        b'{' => b'}',
        b'[' => b']',
        _ => return false,
    };
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (index, &byte) in input.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.pop() != Some(byte) {
                    return false;
                }
                if stack.is_empty() {
                    return byte == expected_root && index + 1 == input.len();
                }
            }
            _ => {}
        }
    }
    false
}

fn trim_bom_and_space(input: &[u8]) -> &[u8] {
    let input = input.trim_ascii();
    input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input)
}

fn gh_wants_data_output(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(*argument, b"--json" | b"--jq")
            || argument.starts_with(b"--json=")
            || argument.starts_with(b"--jq=")
    })
}

fn requests_query(argv: &[&[u8]]) -> bool {
    let command = command_basename(argv[0]);
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version") {
            return true;
        }
        if matches!(*argument, b"-h" | b"-V") && command == b"jq" {
            return true;
        }
    }
    matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
}

fn requests_machine_output(command: &[u8], argv: &[&[u8]]) -> bool {
    match command {
        b"aws" => {
            has_long_option(argv, b"--output")
                || has_long_option(argv, b"--query")
                || has_long_option(argv, b"--cli-binary-format")
                || has_long_option(argv, b"--generate-cli-skeleton")
        }
        b"jq" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"-c"
                    | b"--compact-output"
                    | b"-r"
                    | b"--raw-output"
                    | b"-j"
                    | b"--join-output"
                    | b"--stream"
                    | b"--seq"
            )
        }),
        b"ps" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"-o"
                    | b"-O"
                    | b"-w"
                    | b"-ww"
                    | b"--headers"
                    | b"--no-headers"
                    | b"--format"
                    | b"--cols"
                    | b"--columns"
                    | b"--width"
            ) || argument.starts_with(b"-o")
                || argument.starts_with(b"-O")
                || argument.starts_with(b"--format=")
                || argument.starts_with(b"--cols=")
                || argument.starts_with(b"--columns=")
                || argument.starts_with(b"--width=")
        }),
        b"psql" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"-A"
                    | b"--no-align"
                    | b"-t"
                    | b"--tuples-only"
                    | b"-z"
                    | b"--field-separator-zero"
                    | b"-0"
                    | b"--record-separator-zero"
                    | b"--csv"
                    | b"-H"
                    | b"--html"
                    | b"-x"
                    | b"--expanded"
                    | b"-F"
                    | b"--field-separator"
                    | b"-R"
                    | b"--record-separator"
                    | b"-P"
                    | b"--pset"
            ) || argument.starts_with(b"--field-separator=")
                || argument.starts_with(b"--record-separator=")
                || argument.starts_with(b"--pset=")
                || short_bundle_contains(argument, b"Atz0Hx")
        }),
        b"systemctl" => {
            argv.get(1).is_some_and(|subcommand| {
                matches!(
                    *subcommand,
                    b"show" | b"is-active" | b"is-enabled" | b"is-failed"
                )
            }) || argv[1..].iter().any(|argument| {
                matches!(
                    *argument,
                    b"--property"
                        | b"-p"
                        | b"--value"
                        | b"--no-legend"
                        | b"--plain"
                        | b"--full"
                        | b"--show-types"
                        | b"--output"
                        | b"-o"
                ) || argument.starts_with(b"--property=")
                    || argument.starts_with(b"--output=")
                    || argument.starts_with(b"-p") && argument.len() > 2
                    || argument.starts_with(b"-o") && argument.len() > 2
            })
        }
        b"kubectl" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"--output"
                    | b"-o"
                    | b"--template"
                    | b"--label-columns"
                    | b"-L"
                    | b"--sort-by"
                    | b"--raw"
                    | b"--no-headers"
                    | b"--show-labels"
                    | b"--output-watch-events"
                    | b"--timestamps"
                    | b"--prefix"
            ) || argument.starts_with(b"--output=")
                || argument.starts_with(b"-o") && argument.len() > 2
                || argument.starts_with(b"--template=")
                || argument.starts_with(b"--label-columns=")
                || argument.starts_with(b"-L") && argument.len() > 2
                || argument.starts_with(b"--sort-by=")
        }),
        b"docker" | b"docker-compose" => {
            argv[1..].iter().any(|argument| {
                matches!(*argument, b"--format" | b"-q" | b"--quiet" | b"--no-trunc")
                    || argument.starts_with(b"--format=")
            }) || command == b"docker-compose" && argv.get(1).is_some_and(|arg| *arg == b"config")
                || command == b"docker" && argv.get(1).is_some_and(|arg| *arg == b"inspect")
                || command == b"docker" && argv.get(2).is_some_and(|arg| *arg == b"inspect")
                || command == b"docker"
                    && argv.get(1).is_some_and(|arg| *arg == b"compose")
                    && argv.get(2).is_some_and(|arg| *arg == b"config")
        }
        _ => false,
    }
}

fn has_long_option(argv: &[&[u8]], option: &[u8]) -> bool {
    argv[1..].iter().any(|argument| {
        *argument == option
            || argument
                .strip_prefix(option)
                .is_some_and(|suffix| suffix.starts_with(b"="))
    })
}

fn short_bundle_contains(argument: &[u8], needles: &[u8]) -> bool {
    argument.len() > 2
        && argument[0] == b'-'
        && argument[1] != b'-'
        && argument[1..].iter().any(|byte| needles.contains(byte))
}

pub mod sigil_rle {
    const PREFIX_LEN: usize = 16;
    const SIGIL: u8 = 0x01;

    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut previous_prefix = &[][..];
        let mut first = true;
        let mut offset = 0usize;
        while offset < input.len() {
            let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
            let end = relative_end.map_or(input.len(), |index| offset + index);
            let line = &input[offset..end];
            if line.first() == Some(&SIGIL) {
                output.push(SIGIL);
                output.extend_from_slice(line);
            } else {
                let prefix_len = line.len().min(PREFIX_LEN);
                let prefix = &line[..prefix_len];
                let can_elide = !first
                    && prefix_len == PREFIX_LEN
                    && prefix == previous_prefix
                    && (line.len() == PREFIX_LEN || line[PREFIX_LEN] != SIGIL);
                if can_elide {
                    output.push(SIGIL);
                    output.extend_from_slice(&line[PREFIX_LEN..]);
                } else {
                    output.extend_from_slice(line);
                }
            }
            previous_prefix = &line[..line.len().min(PREFIX_LEN)];
            if end < input.len() {
                output.push(b'\n');
                offset = end + 1;
            } else {
                offset = end;
            }
            first = false;
        }
        output
    }

    pub fn decode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut previous_prefix = [0u8; PREFIX_LEN];
        let mut previous_len = 0usize;
        let mut offset = 0usize;
        while offset < input.len() {
            let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
            let end = relative_end.map_or(input.len(), |index| offset + index);
            let line = &input[offset..end];
            if line.starts_with(&[SIGIL, SIGIL]) {
                let decoded = &line[1..];
                output.extend_from_slice(decoded);
                previous_len = decoded.len().min(PREFIX_LEN);
                previous_prefix[..previous_len].copy_from_slice(&decoded[..previous_len]);
            } else if line.first() == Some(&SIGIL) {
                output.extend_from_slice(&previous_prefix[..previous_len]);
                output.extend_from_slice(&line[1..]);
            } else {
                output.extend_from_slice(line);
                previous_len = line.len().min(PREFIX_LEN);
                previous_prefix[..previous_len].copy_from_slice(&line[..previous_len]);
            }
            if end < input.len() {
                output.push(b'\n');
                offset = end + 1;
            } else {
                offset = end;
            }
        }
        output
    }
}

pub mod ws_rle {
    use super::FilterError;

    const SIGIL: u8 = 0x01;
    const MIN_RUN: usize = 17;
    const MAX_RUN: usize = 255;

    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0usize;
        while index < input.len() {
            if input[index] == SIGIL {
                output.extend_from_slice(&[SIGIL, 0]);
                index += 1;
            } else if input[index] == b' ' {
                let mut after = index;
                while after < input.len() && input[after] == b' ' {
                    after += 1;
                }
                let mut run = after - index;
                if run < MIN_RUN {
                    output.extend_from_slice(&input[index..after]);
                } else {
                    while run > 0 {
                        let chunk = run.min(MAX_RUN);
                        if chunk < MIN_RUN {
                            output.extend(std::iter::repeat_n(b' ', chunk));
                        } else {
                            output.extend_from_slice(&[SIGIL, chunk as u8]);
                        }
                        run -= chunk;
                    }
                }
                index = after;
            } else {
                output.push(input[index]);
                index += 1;
            }
        }
        output
    }

    pub fn decode(input: &[u8]) -> Result<Vec<u8>, FilterError> {
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0usize;
        while index < input.len() {
            if input[index] != SIGIL {
                output.push(input[index]);
                index += 1;
                continue;
            }
            let length = *input.get(index + 1).ok_or(FilterError::InvalidInput)?;
            if length == 0 {
                output.push(SIGIL);
            } else if usize::from(length) >= MIN_RUN {
                output.extend(std::iter::repeat_n(b' ', usize::from(length)));
            } else {
                return Err(FilterError::InvalidInput);
            }
            index += 2;
        }
        Ok(output)
    }
}
