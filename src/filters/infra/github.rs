use super::*;

pub(super) fn compact_gh(argv: &[&[u8]], stdout: &[u8]) -> Vec<u8> {
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
