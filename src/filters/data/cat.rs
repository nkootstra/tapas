use super::*;

pub(super) fn compact_cat(input: &[u8], argv: &[&[u8]]) -> Option<Vec<u8>> {
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

pub(super) fn cat_requests_exact(argv: &[&[u8]]) -> bool {
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
