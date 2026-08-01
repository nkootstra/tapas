use super::{EvidenceClass, FilterError, StreamFilterOutput};

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    let Some(command) = argv.first().copied().map(command_basename) else {
        return Err(FilterError::InvalidInput);
    };
    if lossless || requests_exact_output(command, argv) {
        return Ok(passthrough(stdout, stderr));
    }
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    let arg3 = argv.get(3).copied().unwrap_or_default();

    if exit_code != 0 && !stderr.is_empty() {
        return Ok(passthrough(stdout, stderr));
    }

    let output = if command == b"curl" && has_verbose_flag(argv) {
        Some(compact_curl(stdout, stderr))
    } else if is_logs_invocation(command, argv) {
        let compose = command == b"docker-compose" || command == b"docker" && arg1 == b"compose";
        Some(compact_logs(stdout, stderr, compose))
    } else if is_docker_ps(command, argv) && matches_docker_ps(stdout) {
        Some(compact_docker_ps(stdout))
    } else if is_docker_images(command, argv) && matches_docker_images(stdout) {
        Some(compact_docker_images(stdout))
    } else if command == b"kubectl" && matches_kubectl(stdout) {
        Some(compact_kubectl(stdout))
    } else if command == b"gh" {
        Some(compact_gh(argv, stdout))
    } else if command == b"acli" {
        compact_acli(arg1, arg2, arg3, stdout)
    } else {
        None
    };

    Ok(output.map_or_else(
        || passthrough(stdout, stderr),
        |stdout| StreamFilterOutput::new(stdout, Vec::new(), EvidenceClass::FactComplete),
    ))
}

fn is_logs_invocation(command: &[u8], argv: &[&[u8]]) -> bool {
    command == b"kubectl" && argv.get(1).copied() == Some(b"logs")
        || command == b"docker" && argv.get(1).copied() == Some(b"logs")
        || command == b"docker-compose" && argv.get(1).copied() == Some(b"logs")
        || command == b"docker"
            && argv.get(1).copied() == Some(b"compose")
            && argv.get(2).copied() == Some(b"logs")
}

fn compact_logs(stdout: &[u8], stderr: &[u8], compose: bool) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_logs(stdout, &mut output, compose);
    scan_logs(stderr, &mut output, compose);
    output
}

fn scan_logs(input: &[u8], output: &mut Vec<u8>, compose: bool) {
    let mut pending = Vec::new();
    let mut pending_fingerprint = Vec::new();
    let mut repeats = 0usize;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if line.is_empty() {
            flush_log(output, &mut pending, &mut pending_fingerprint, &mut repeats);
            continue;
        }
        let normalized = normalize_log_line(line, compose);
        let fingerprint = if compose {
            normalized.clone()
        } else {
            normalized[timestamp_end(&normalized)..].to_vec()
        };
        if repeats > 0 && fingerprint == pending_fingerprint {
            repeats += 1;
            continue;
        }
        flush_log(output, &mut pending, &mut pending_fingerprint, &mut repeats);
        pending = normalized;
        pending_fingerprint = fingerprint;
        repeats = 1;
    }
    flush_log(output, &mut pending, &mut pending_fingerprint, &mut repeats);
}

fn normalize_log_line(line: &[u8], compose: bool) -> Vec<u8> {
    if compose && let Some(pipe) = line.iter().position(|byte| *byte == b'|') {
        let service = line[..pipe].trim_ascii();
        if !service.is_empty() {
            let payload = line[pipe + 1..].trim_ascii_start();
            let payload = &payload[timestamp_end(payload)..];
            let mut result = service.to_vec();
            result.push(b'|');
            if !payload.is_empty() {
                result.push(b' ');
                result.extend_from_slice(payload);
            }
            return result;
        }
    }
    line.to_vec()
}

fn flush_log(
    output: &mut Vec<u8>,
    pending: &mut Vec<u8>,
    fingerprint: &mut Vec<u8>,
    repeats: &mut usize,
) {
    if *repeats == 0 {
        return;
    }
    let start = timestamp_end(pending);
    output.extend_from_slice(if start > 0 {
        &pending[start..]
    } else {
        pending
    });
    if *repeats > 1 {
        output.extend_from_slice(" ×".as_bytes());
        output.extend_from_slice(repeats.to_string().as_bytes());
    }
    output.push(b'\n');
    pending.clear();
    fingerprint.clear();
    *repeats = 0;
}

fn timestamp_end(line: &[u8]) -> usize {
    if !looks_like_date(line) {
        return 0;
    }
    let mut cursor = if line.get(10) == Some(&b' ') && looks_like_clock(&line[11..]) {
        19
    } else {
        0
    };
    while cursor < line.len() && !matches!(line[cursor], b' ' | b'\t') {
        cursor += 1;
    }
    while line
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    cursor
}

fn looks_like_date(line: &[u8]) -> bool {
    line.len() >= 10
        && line[..4].iter().all(u8::is_ascii_digit)
        && line[4] == b'-'
        && line[5..7].iter().all(u8::is_ascii_digit)
        && line[7] == b'-'
        && line[8..10].iter().all(u8::is_ascii_digit)
}

fn looks_like_clock(line: &[u8]) -> bool {
    line.len() >= 8
        && line[..2].iter().all(u8::is_ascii_digit)
        && line[2] == b':'
        && line[3..5].iter().all(u8::is_ascii_digit)
        && line[5] == b':'
        && line[6..8].iter().all(u8::is_ascii_digit)
}

fn is_docker_ps(command: &[u8], argv: &[&[u8]]) -> bool {
    command == b"docker" && argv.get(1).copied() == Some(b"ps")
        || command == b"docker-compose" && argv.get(1).copied() == Some(b"ps")
        || command == b"docker"
            && argv.get(1).copied() == Some(b"compose")
            && argv.get(2).copied() == Some(b"ps")
}

fn matches_docker_ps(input: &[u8]) -> bool {
    first_nonempty(input).is_some_and(|header| {
        header.starts_with(b"CONTAINER ID")
            || header.starts_with(b"NAME")
                && find_subslice(header, b"IMAGE").is_some()
                && find_subslice(header, b"SERVICE").is_some()
                && find_subslice(header, b"STATUS").is_some()
    })
}

fn compact_docker_ps(input: &[u8]) -> Vec<u8> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let header = lines.next().unwrap_or_default();
    let status_column = find_subslice(header, b"STATUS").unwrap_or(0);
    let names_column = find_subslice(header, b"NAMES")
        .or_else(|| find_subslice(header, b"NAME"))
        .unwrap_or(0);
    let compose = names_column == 0 && header.starts_with(b"NAME");
    let rows = lines.filter(|line| !line.is_empty()).collect::<Vec<_>>();
    let up = rows
        .iter()
        .filter(|line| {
            line.get(status_column..)
                .is_some_and(|status| status.trim_ascii_start().starts_with(b"Up"))
        })
        .count();
    let state: &[u8] = if rows.is_empty() || up == 0 {
        b"none".as_slice()
    } else if up == rows.len() {
        b"up"
    } else {
        b"m"
    };
    let mut output = b"d".to_vec();
    output.extend_from_slice(rows.len().to_string().as_bytes());
    output.extend_from_slice(state);
    for line in rows {
        let name = if compose {
            first_field(line)
        } else if names_column > 0 && names_column < line.len() {
            line[names_column..].trim_ascii()
        } else {
            last_field(line)
        };
        if name.is_empty() {
            continue;
        }
        output.push(b' ');
        output.extend_from_slice(name);
        if compose {
            let rest = line
                .trim_ascii_start()
                .get(name.len()..)
                .unwrap_or_default();
            let image = first_field(rest);
            if !image.is_empty() {
                output.push(b'(');
                output.extend_from_slice(image);
                output.push(b')');
            }
        }
    }
    output.push(b'\n');
    output
}

fn is_docker_images(command: &[u8], argv: &[&[u8]]) -> bool {
    command == b"docker" && matches!(argv.get(1), Some(&b"images") | Some(&b"image"))
}

fn matches_docker_images(input: &[u8]) -> bool {
    first_nonempty(input).is_some_and(|header| {
        header.starts_with(b"REPOSITORY")
            && find_subslice(header, b"TAG").is_some()
            && find_subslice(header, b"IMAGE ID").is_some()
            && find_subslice(header, b"SIZE").is_some()
    })
}

fn compact_docker_images(input: &[u8]) -> Vec<u8> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let header = lines.next().unwrap_or_default();
    let tag = find_subslice(header, b"TAG").unwrap_or(0);
    let image_id = find_subslice(header, b"IMAGE ID").unwrap_or(0);
    let size = find_subslice(header, b"SIZE").unwrap_or(0);
    let mut named = Vec::<(&[u8], &[u8], &[u8])>::new();
    let mut dangling = 0usize;
    for line in lines.filter(|line| !line.is_empty()) {
        if tag >= line.len() || image_id > line.len() || size >= line.len() {
            continue;
        }
        let repository = line[..tag].trim_ascii();
        let version = line[tag..image_id].trim_ascii();
        let bytes = line[size..].trim_ascii();
        if repository.is_empty() || version.is_empty() || bytes.is_empty() {
            continue;
        }
        if repository == b"<none>" || version == b"<none>" {
            dangling += 1;
        } else {
            named.push((repository, version, bytes));
        }
    }
    let total = named.len() + dangling;
    let mut output = b"images ".to_vec();
    output.extend_from_slice(total.to_string().as_bytes());
    if total > 0 {
        output.push(b':');
        for (repository, version, bytes) in named.iter().take(8) {
            output.push(b' ');
            output.extend_from_slice(repository);
            output.push(b':');
            output.extend_from_slice(version);
            output.push(b'(');
            output.extend_from_slice(bytes);
            output.push(b')');
        }
        if dangling > 0 {
            output.extend_from_slice(b" dangling x");
            output.extend_from_slice(dangling.to_string().as_bytes());
        }
        if named.len() > 8 {
            output.extend_from_slice(b" (+");
            output.extend_from_slice((named.len() - 8).to_string().as_bytes());
            output.push(b')');
        }
    }
    output.push(b'\n');
    output
}

fn matches_kubectl(input: &[u8]) -> bool {
    first_nonempty(input).is_some_and(|header| {
        header.starts_with(b"NAME")
            && find_subslice(header, b"READY").is_some()
            && find_subslice(header, b"STATUS").is_some()
    })
}

fn compact_kubectl(input: &[u8]) -> Vec<u8> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let header = lines.next().unwrap_or_default();
    let ready_column = find_subslice(header, b"READY").unwrap_or(0);
    let status_column = find_subslice(header, b"STATUS").unwrap_or(0);
    let rows = lines.filter(|line| !line.is_empty()).collect::<Vec<_>>();
    let healthy = rows
        .iter()
        .filter(|line| healthy_pod(line, ready_column, status_column))
        .count();
    let aggregate = if rows.is_empty() || healthy == 0 {
        b'n'
    } else if healthy == rows.len() {
        b'r'
    } else {
        b'm'
    };
    let mut output = b"k".to_vec();
    output.extend_from_slice(rows.len().to_string().as_bytes());
    output.push(aggregate);
    for line in rows {
        let name = first_field(line);
        if name.is_empty() {
            continue;
        }
        output.push(b' ');
        output.extend_from_slice(name);
        if aggregate == b'r' {
            continue;
        }
        let ready = field_at(line, ready_column);
        output.push(b'(');
        output.extend_from_slice(ready);
        if !healthy_pod(line, ready_column, status_column) {
            output.push(b',');
            output.extend_from_slice(field_at(line, status_column));
        }
        output.push(b')');
    }
    output.push(b'\n');
    output
}

fn healthy_pod(line: &[u8], ready: usize, status: usize) -> bool {
    field_at(line, status) == b"Running" && ready_full(field_at(line, ready))
}

fn ready_full(ready: &[u8]) -> bool {
    let Some(slash) = ready.iter().position(|byte| *byte == b'/') else {
        return false;
    };
    let (left, right) = (&ready[..slash], &ready[slash + 1..]);
    !left.is_empty() && left != b"0" && left == right
}

fn field_at(line: &[u8], column: usize) -> &[u8] {
    if column >= line.len() {
        return b"";
    }
    let mut cursor = column;
    while line
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    if column > 0 && cursor == column && !matches!(line[column], b' ' | b'\t') {
        while cursor > 0 && !matches!(line[cursor - 1], b' ' | b'\t') {
            cursor -= 1;
        }
    }
    let start = cursor;
    while line
        .get(cursor)
        .is_some_and(|byte| !matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    &line[start..cursor]
}

fn compact_curl(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut method = b"request".as_slice();
    let mut path = b"/".as_slice();
    let mut host = b"unknown".as_slice();
    let mut status = b"response".as_slice();
    let mut content_type = b"".as_slice();
    let mut content_length = b"".as_slice();
    for raw in stderr.split(|byte| *byte == b'\n') {
        let line = raw.trim_ascii();
        if let Some(request) = line.strip_prefix(b">") {
            let fields = request
                .trim_ascii()
                .split(|byte| *byte == b' ')
                .collect::<Vec<_>>();
            if fields.len() >= 2 && fields[0].iter().all(u8::is_ascii_uppercase) {
                method = fields[0];
                path = fields[1];
            }
        }
        if let Some(value) = strip_prefix_ignore_ascii_case(line, b"> Host:") {
            host = value.trim_ascii();
        }
        if line.starts_with(b"< HTTP/") {
            status = line[2..].trim_ascii();
        }
        if let Some(value) = strip_prefix_ignore_ascii_case(line, b"< content-type:") {
            content_type = value
                .trim_ascii()
                .split(|byte| *byte == b';')
                .next()
                .unwrap_or_default();
        }
        if let Some(value) = strip_prefix_ignore_ascii_case(line, b"< content-length:") {
            content_length = value.trim_ascii();
        }
    }
    let mut output = b"curl ".to_vec();
    output.extend_from_slice(method);
    output.push(b' ');
    output.extend_from_slice(host);
    output.extend_from_slice(path);
    output.extend_from_slice(b" -> ");
    output.extend_from_slice(status);
    if !content_type.is_empty() {
        output.push(b' ');
        output.extend_from_slice(content_type);
    }
    if !content_length.is_empty() {
        output.extend_from_slice(b" len=");
        output.extend_from_slice(content_length);
    }
    output.push(b'\n');
    output.extend_from_slice(stdout);
    output
}

fn has_verbose_flag(argv: &[&[u8]]) -> bool {
    argv[1..]
        .iter()
        .take_while(|arg| **arg != b"--")
        .any(|arg| **arg == *b"--verbose" || arg.starts_with(b"-") && arg[1..].contains(&b'v'))
}

fn compact_gh(argv: &[&[u8]], stdout: &[u8]) -> Vec<u8> {
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    if arg1 == b"pr" && arg2 == b"view" {
        return compact_gh_pr_view(stdout);
    }
    if arg1 == b"pr" && arg2 == b"checks" {
        return compact_gh_checks(stdout);
    }
    if arg1 == b"run" && arg2 == b"view" {
        return compact_gh_run_view(stdout);
    }
    if arg1 == b"pr" && arg2 == b"list" {
        let mut output = Vec::with_capacity(stdout.len());
        for line in stdout.split(|byte| *byte == b'\n') {
            let trimmed = line.trim_ascii();
            if find_subslice(trimmed, b"pull request").is_some() || trimmed.starts_with(b"#") {
                append_line(&mut output, line.trim_ascii_end());
            }
        }
        return output;
    }
    if arg1 == b"run" && arg2 == b"list" {
        return collapse_table(stdout);
    }
    stdout.to_vec()
}

fn compact_gh_pr_view(stdout: &[u8]) -> Vec<u8> {
    let lines = stdout.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    if !lines
        .first()
        .is_some_and(|line| metadata_key(line).is_some())
    {
        return stdout.to_vec();
    }
    let Some(separator) = lines.iter().position(|line| *line == b"--") else {
        return stdout.to_vec();
    };
    let value = |wanted: &[u8]| {
        lines[..separator].iter().find_map(|line| {
            let key = metadata_key(line)?;
            (key == wanted).then_some(&line[key.len() + 2..])
        })
    };
    let mut output = Vec::with_capacity(stdout.len());
    if let Some(number) = value(b"number") {
        output.push(b'#');
        output.extend_from_slice(number);
    }
    for field in [b"state".as_slice(), b"title"] {
        if let Some(value) = value(field) {
            if !output.is_empty() {
                output.push(b' ');
            }
            output.extend_from_slice(value);
        }
    }
    if !output.is_empty() {
        output.push(b'\n');
    }
    for line in &lines[..separator] {
        if let Some(key) = metadata_key(line) {
            let value = &line[key.len() + 2..];
            if !matches!(key, b"number" | b"state" | b"title") && !value.trim_ascii().is_empty() {
                append_line(&mut output, line);
            }
        }
    }
    for (index, line) in lines[separator + 1..].iter().enumerate() {
        output.extend_from_slice(line);
        if index + separator + 2 < lines.len() {
            output.push(b'\n');
        }
    }
    output
}

fn metadata_key(line: &[u8]) -> Option<&[u8]> {
    let colon = line.iter().position(|byte| *byte == b':')?;
    if colon == 0 || line.get(colon + 1) != Some(&b'\t') {
        return None;
    }
    let key = &line[..colon];
    key.iter()
        .all(|byte| byte.is_ascii_lowercase() || *byte == b'-')
        .then_some(key)
}

fn compact_gh_checks(stdout: &[u8]) -> Vec<u8> {
    let mut states = Vec::<(Vec<u8>, usize)>::new();
    let mut details = Vec::new();
    let mut total = 0usize;
    for line in stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let fields = line.split(|byte| *byte == b'\t').collect::<Vec<_>>();
        if fields.len() < 2 {
            return stdout.to_vec();
        }
        total += 1;
        if let Some((_, count)) = states.iter_mut().find(|(state, _)| state == fields[1]) {
            *count += 1;
        } else {
            states.push((fields[1].to_vec(), 1));
        }
        if fields[1] != b"pass" {
            append_line(&mut details, line);
        }
    }
    if total == 0 {
        return stdout.to_vec();
    }
    let mut output = total.to_string().into_bytes();
    output.extend_from_slice(b" checks:");
    for (index, (state, count)) in states.iter().enumerate() {
        output.extend_from_slice(if index == 0 { b" " } else { b", " });
        output.extend_from_slice(count.to_string().as_bytes());
        output.push(b' ');
        output.extend_from_slice(state);
    }
    output.push(b'\n');
    output.extend_from_slice(&details);
    output
}

fn compact_gh_run_view(stdout: &[u8]) -> Vec<u8> {
    let lines = stdout.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    let Some(mut cursor) = lines.iter().position(|line| *line == b"JOBS") else {
        return stdout.to_vec();
    };
    let mut output = Vec::with_capacity(stdout.len());
    for line in &lines[..cursor] {
        if !line.is_empty() {
            append_line(&mut output, line);
        }
    }
    output.extend_from_slice(b"JOBS\n");
    cursor += 1;
    emit_gh_jobs(&lines, &mut cursor, &mut output);

    while cursor < lines.len() {
        if lines[cursor] == b"ANNOTATIONS" {
            output.extend_from_slice(b"ANNOTATIONS\n");
            cursor += 1;
            emit_gh_annotations(&lines, &mut cursor, &mut output);
        } else if lines[cursor] == b"ARTIFACTS" {
            output.extend_from_slice(b"ARTIFACTS\n");
            cursor += 1;
            while cursor < lines.len() && !gh_section(lines[cursor]) && !gh_footer(lines[cursor]) {
                if !lines[cursor].is_empty() {
                    append_line(&mut output, lines[cursor]);
                }
                cursor += 1;
            }
        } else {
            let line = lines[cursor];
            if !line.is_empty() && gh_footer(line) && !line.starts_with(b"For more information") {
                append_line(&mut output, line);
            }
            cursor += 1;
        }
    }
    output
}

fn emit_gh_jobs(lines: &[&[u8]], cursor: &mut usize, output: &mut Vec<u8>) {
    let mut passed = 0usize;
    let mut details = Vec::new();
    while *cursor < lines.len() && !gh_section(lines[*cursor]) && !gh_footer(lines[*cursor]) {
        let line = lines[*cursor];
        if line.is_empty() {
            *cursor += 1;
            continue;
        }
        if line.starts_with(b"  ") {
            append_line(&mut details, line);
            *cursor += 1;
            continue;
        }
        if line.starts_with("✓ ".as_bytes()) {
            passed += 1;
            *cursor += 1;
            continue;
        }
        append_line(&mut details, line);
        *cursor += 1;
        let mut passed_steps = 0usize;
        let mut kept_steps = Vec::new();
        while *cursor < lines.len() && lines[*cursor].starts_with(b"  ") {
            let step = lines[*cursor].trim_ascii_start();
            if step.starts_with("✓ ".as_bytes()) {
                passed_steps += 1;
            } else {
                kept_steps.push(lines[*cursor]);
            }
            *cursor += 1;
        }
        if passed_steps > 0 {
            details.extend_from_slice("  ✓ ".as_bytes());
            details.extend_from_slice(passed_steps.to_string().as_bytes());
            details.extend_from_slice(b" steps passed\n");
        }
        for step in kept_steps {
            append_line(&mut details, step);
        }
    }
    if passed > 0 {
        output.extend_from_slice("✓ ".as_bytes());
        output.extend_from_slice(passed.to_string().as_bytes());
        output.extend_from_slice(b" passed\n");
    }
    output.extend_from_slice(&details);
}

#[derive(Debug)]
struct GhAnnotation {
    message: Vec<u8>,
    locations: Vec<Vec<u8>>,
}

fn emit_gh_annotations(lines: &[&[u8]], cursor: &mut usize, output: &mut Vec<u8>) {
    let mut groups = Vec::<GhAnnotation>::new();
    while *cursor < lines.len() && !gh_section(lines[*cursor]) && !gh_footer(lines[*cursor]) {
        let line = lines[*cursor];
        if line.is_empty() {
            *cursor += 1;
            continue;
        }
        if !line.starts_with("! ".as_bytes()) && !line.starts_with(b"X ") {
            append_line(output, line);
            *cursor += 1;
            continue;
        }
        *cursor += 1;
        let mut location = None;
        if *cursor < lines.len()
            && !lines[*cursor].is_empty()
            && !lines[*cursor].starts_with("! ".as_bytes())
            && !lines[*cursor].starts_with(b"X ")
            && !gh_section(lines[*cursor])
            && !gh_footer(lines[*cursor])
        {
            let raw = lines[*cursor];
            let job = raw
                .windows(2)
                .rposition(|window| window == b": ")
                .map_or(raw, |index| &raw[..index]);
            location = Some(job.to_vec());
            *cursor += 1;
        }
        if let Some(group) = groups.iter_mut().find(|group| group.message == line) {
            if let Some(location) = location {
                group.locations.push(location);
            }
        } else {
            groups.push(GhAnnotation {
                message: line.to_vec(),
                locations: location.into_iter().collect(),
            });
        }
    }
    for group in groups {
        output.extend_from_slice(&group.message);
        match group.locations.as_slice() {
            [] => {}
            [location] => {
                output.extend_from_slice(b"  [");
                output.extend_from_slice(location);
                output.push(b']');
            }
            locations => {
                output.extend_from_slice("  [×".as_bytes());
                output.extend_from_slice(locations.len().to_string().as_bytes());
                output.extend_from_slice(b": ");
                for (index, location) in locations.iter().enumerate() {
                    if index > 0 {
                        output.extend_from_slice(b", ");
                    }
                    output.extend_from_slice(location);
                }
                output.push(b']');
            }
        }
        output.push(b'\n');
    }
}

fn gh_section(line: &[u8]) -> bool {
    matches!(line, b"JOBS" | b"ANNOTATIONS" | b"ARTIFACTS")
}

fn gh_footer(line: &[u8]) -> bool {
    line.starts_with(b"For more information")
        || line.starts_with(b"To see what failed")
        || line.starts_with(b"View this run on GitHub")
}

fn compact_acli(arg1: &[u8], arg2: &[u8], arg3: &[u8], stdout: &[u8]) -> Option<Vec<u8>> {
    let table = arg1 == b"jira" && arg2 == b"workitem" && arg3 == b"search"
        || arg1 == b"confluence" && arg2 == b"space" && arg3 == b"list";
    if table {
        return Some(collapse_table(stdout));
    }
    let view = arg1 == b"jira" && arg2 == b"workitem" && arg3 == b"view"
        || arg1 == b"confluence" && arg2 == b"page" && arg3 == b"view";
    view.then(|| compact_acli_view(stdout))
}

fn compact_acli_view(stdout: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len());
    let mut kept = 0usize;
    let mut pending_body = false;
    let mut pending_field = false;
    let mut in_fields = false;
    for raw in stdout.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if pending_body {
            pending_body = false;
            if !is_body_boundary(line) {
                output.extend_from_slice(b"  ");
                append_line(&mut output, line);
                kept += 1;
                continue;
            }
        }
        if pending_field {
            pending_field = false;
            if !looks_like_label(line) {
                output.extend_from_slice(b"  ");
                append_line(&mut output, line);
                kept += 1;
                continue;
            }
        }
        if line.starts_with(b"Fields:") {
            in_fields = true;
            append_line(&mut output, line);
            kept += 1;
        } else if is_body_label(line) {
            in_fields = false;
            append_line(&mut output, line);
            kept += 1;
            pending_body = line.ends_with(b":");
        } else if in_fields && looks_like_label(line) {
            append_line(&mut output, line);
            kept += 1;
            pending_field = line.ends_with(b":");
        } else if is_acli_metadata(line) {
            append_line(&mut output, line);
            kept += 1;
        }
    }
    if kept == 0 { stdout.to_vec() } else { output }
}

fn is_acli_metadata(line: &[u8]) -> bool {
    [
        b"Key:".as_slice(),
        b"Work item:",
        b"Issue:",
        b"Type:",
        b"Summary:",
        b"Status:",
        b"Assignee:",
        b"Priority:",
        b"Reporter:",
        b"Created:",
        b"Updated:",
        b"URL:",
        b"Web URL:",
        b"ID:",
        b"Title:",
        b"Space:",
        b"Author:",
        b"Created by:",
        b"Last updated:",
        b"Version:",
        b"Labels:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn is_body_label(line: &[u8]) -> bool {
    [b"Description:".as_slice(), b"Body:", b"Comments:"]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

fn is_body_boundary(line: &[u8]) -> bool {
    is_body_label(line) || line.starts_with(b"Fields:")
}

fn looks_like_label(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|byte| *byte == b':') else {
        return false;
    };
    colon > 0
        && colon <= 48
        && line[..colon].iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b' ' | b'-' | b'_' | b'/' | b'&' | b'(' | b')')
        })
}

fn collapse_table(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let mut first = true;
        for field in line
            .split(|byte| matches!(byte, b' ' | b'\t'))
            .filter(|field| !field.is_empty())
        {
            if !first {
                output.push(b' ');
            }
            first = false;
            output.extend_from_slice(field);
        }
        output.push(b'\n');
    }
    output
}

fn requests_exact_output(command: &[u8], argv: &[&[u8]]) -> bool {
    argv[1..]
        .iter()
        .take_while(|arg| **arg != b"--")
        .any(|arg| {
            matches!(*arg, b"--help" | b"--version" | b"-h" | b"-V")
                || long_option(arg, b"--format")
                || long_option(arg, b"--output")
                || arg.starts_with(b"-o") && command == b"kubectl"
                || long_option(arg, b"--json")
                || long_option(arg, b"--jq")
                || long_option(arg, b"--template")
        })
}

fn long_option(argument: &[u8], option: &[u8]) -> bool {
    argument == option
        || argument
            .strip_prefix(option)
            .is_some_and(|rest| rest.starts_with(b"="))
}

fn first_nonempty(input: &[u8]) -> Option<&[u8]> {
    input
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
}

fn first_field(line: &[u8]) -> &[u8] {
    let line = line.trim_ascii_start();
    &line[..line
        .iter()
        .position(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
        .unwrap_or(line.len())]
}

fn last_field(line: &[u8]) -> &[u8] {
    let line = line.trim_ascii_end();
    for index in (2..=line.len()).rev() {
        if line[index - 2..index] == *b"  " {
            return &line[index..];
        }
    }
    line
}

fn append_line(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(line);
    output.push(b'\n');
}

fn strip_prefix_ignore_ascii_case<'a>(input: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    input
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &input[prefix.len()..])
}

fn strip_ansi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        if input[cursor] == 0x1b && input.get(cursor + 1) == Some(&b'[') {
            cursor += 2;
            while cursor < input.len() {
                let byte = input[cursor];
                cursor += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else {
            output.push(input[cursor]);
            cursor += 1;
        }
    }
    output
}

fn command_basename(command: &[u8]) -> &[u8] {
    command
        .iter()
        .rposition(|byte| matches!(byte, b'/' | b'\\'))
        .map_or(command, |separator| &command[separator + 1..])
}

fn passthrough(stdout: &[u8], stderr: &[u8]) -> StreamFilterOutput {
    StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (!needle.is_empty() && needle.len() <= haystack.len())
        .then(|| {
            haystack
                .windows(needle.len())
                .position(|window| window == needle)
        })
        .flatten()
}
