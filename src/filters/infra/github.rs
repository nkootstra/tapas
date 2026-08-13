pub(super) fn compact_gh(argv: &[&[u8]], stdout: &[u8]) -> Option<Vec<u8>> {
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    // Machine and caller-shaped output is an exact contract. In particular,
    // parsing and reserializing JSON can change bytes while preserving data.
    if has_json_selection(argv) || arg1 == b"api" {
        return None;
    }
    if arg1 == b"pr" && arg2 == b"view" {
        return compact_gh_detail_view(stdout);
    }
    if arg1 == b"issue" && arg2 == b"view" {
        return compact_gh_detail_view(stdout);
    }
    if arg1 == b"pr" && arg2 == b"checks" {
        return compact_gh_checks(stdout);
    }
    if arg1 == b"run" && arg2 == b"view" {
        return Some(compact_gh_run_view(stdout));
    }
    if arg1 == b"pr" && arg2 == b"list" {
        let mut output = Vec::with_capacity(stdout.len());
        for line in stdout.split(|byte| *byte == b'\n') {
            let trimmed = line.trim_ascii();
            if find_subslice(trimmed, b"pull request").is_some() || trimmed.starts_with(b"#") {
                append_line(&mut output, line.trim_ascii_end());
            }
        }
        return (!output.is_empty()).then_some(output);
    }
    if arg1 == b"run" && arg2 == b"list" {
        return compact_gh_run_list(stdout);
    }
    // Tabular listings keep every row; only alignment padding is collapsed.
    if arg1 == b"search"
        || (arg1 == b"release" && arg2 == b"list")
        || (arg1 == b"issue" && arg2 == b"list")
    {
        return compact_gh_table(stdout);
    }
    None
}

fn has_json_selection(argv: &[&[u8]]) -> bool {
    crate::invocation_policy::options(argv)
        .iter()
        .any(|argument| *argument == b"--json" || argument.starts_with(b"--json="))
}

fn compact_gh_table(input: &[u8]) -> Option<Vec<u8>> {
    let lines = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.trim_ascii().is_empty())
        .collect::<Vec<_>>();
    let header = lines.first()?.trim_ascii_end();
    let starts = column_starts(header);
    if starts.len() < 2 {
        return None;
    }

    let mut output = Vec::with_capacity(input.len());
    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim_ascii_end();
        if index > 0 && (line.starts_with(b"Showing ") || line.starts_with(b"No ")) {
            output.extend_from_slice(&collapse_table(line));
            continue;
        }
        let row_starts = if index == 0 {
            starts.clone()
        } else {
            row_starts(line, &starts)
        };
        for (field_index, start) in row_starts.iter().enumerate() {
            if field_index > 0 {
                output.push(b'\t');
            }
            let end = row_starts
                .get(field_index + 1)
                .copied()
                .unwrap_or(line.len())
                .min(line.len());
            if *start < end {
                output.extend_from_slice(line[*start..end].trim_ascii());
            }
        }
        output.push(b'\n');
    }
    Some(output)
}

fn row_starts(line: &[u8], header_starts: &[usize]) -> Vec<usize> {
    let mut starts = vec![header_starts.first().copied().unwrap_or(0)];
    let mut cursor = starts[0];
    for field_index in 1..header_starts.len() {
        let nominal = header_starts[field_index];
        let start = nearest_column_boundary(
            line,
            cursor,
            nominal,
            header_starts.get(field_index + 1).copied(),
            header_starts.len() - field_index,
        )
        .unwrap_or(nominal);
        starts.push(start);
        cursor = start;
    }
    starts
}

fn nearest_column_boundary(
    line: &[u8],
    cursor: usize,
    nominal: usize,
    next_nominal: Option<usize>,
    boundaries_remaining: usize,
) -> Option<usize> {
    let mut index = cursor.min(line.len());
    let mut before = None;
    let mut aligned = None;
    let mut within = None;
    let mut within_count = 0;
    let mut crossing = None;
    let mut overflow = Vec::new();
    let mut after = None;
    while index < line.len() {
        if line[index].is_ascii_whitespace() {
            let start = index;
            while index < line.len() && line[index].is_ascii_whitespace() {
                index += 1;
            }
            if index - start >= 2 && index < line.len() {
                if next_nominal.is_some_and(|next| start <= nominal && index >= next) {
                    return Some(nominal.max(cursor));
                }
                if start < nominal && index >= nominal {
                    aligned.get_or_insert(index);
                } else if index <= nominal {
                    before = Some(index);
                } else if let Some(next) = next_nominal {
                    if index <= next {
                        within = Some(index);
                        within_count += 1;
                    } else if start >= next {
                        overflow.push(index);
                    } else {
                        crossing.get_or_insert(index);
                    }
                } else {
                    after.get_or_insert(index);
                }
            }
        } else {
            index += 1;
        }
    }
    if aligned.is_some() {
        return aligned;
    }
    if next_nominal.is_some() {
        if crossing.is_some() {
            return before.or(crossing);
        }
        let candidate_count = within_count + overflow.len();
        if candidate_count > boundaries_remaining {
            let skipped = candidate_count - boundaries_remaining;
            if skipped >= within_count {
                return overflow
                    .get(skipped - within_count)
                    .copied()
                    .or_else(|| overflow.last().copied());
            }
        }
        return within.or_else(|| overflow.first().copied()).or(before);
    }
    after.or(before)
}

fn column_starts(header: &[u8]) -> Vec<usize> {
    let first = header
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(header.len());
    if first == header.len() {
        return Vec::new();
    }
    let mut starts = vec![first];
    let mut index = first;
    while index + 1 < header.len() {
        if header[index].is_ascii_whitespace() && header[index + 1].is_ascii_whitespace() {
            let mut next = index + 2;
            while next < header.len() && header[next].is_ascii_whitespace() {
                next += 1;
            }
            if next < header.len() {
                starts.push(next);
                index = next;
                continue;
            }
        }
        index += 1;
    }
    starts
}

fn compact_gh_detail_view(stdout: &[u8]) -> Option<Vec<u8>> {
    let lines = stdout.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    if !lines
        .first()
        .is_some_and(|line| metadata_key(line).is_some())
    {
        return None;
    }
    let separator = lines.iter().position(|line| *line == b"--")?;
    if separator == 0
        || lines[..separator]
            .iter()
            .any(|line| metadata_key(line).is_none())
    {
        return None;
    }
    let value = |wanted: &[u8]| {
        let mut matches = lines[..separator].iter().filter_map(|line| {
            let key = metadata_key(line)?;
            (key == wanted).then_some(&line[key.len() + 2..])
        });
        let value = matches.next()?;
        matches.next().is_none().then_some(value)
    };
    let (Some(number), Some(state), Some(title)) =
        (value(b"number"), value(b"state"), value(b"title"))
    else {
        return None;
    };
    if number.is_empty()
        || !number.iter().all(u8::is_ascii_digit)
        || state.is_empty()
        || title.is_empty()
    {
        return None;
    }
    let mut output = Vec::with_capacity(stdout.len());
    output.push(b'#');
    output.extend_from_slice(number);
    output.push(b' ');
    output.extend_from_slice(state);
    output.push(b' ');
    output.extend_from_slice(title);
    output.push(b'\n');
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
    Some(output)
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

fn compact_gh_checks(stdout: &[u8]) -> Option<Vec<u8>> {
    let mut states = Vec::<(Vec<u8>, usize)>::new();
    let mut details = Vec::new();
    let mut total = 0usize;
    for line in stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let fields = line.split(|byte| *byte == b'\t').collect::<Vec<_>>();
        if !matches!(fields.as_slice(), [name, state, duration, url] | [name, state, duration, url, b""]
            if !name.is_empty()
                && matches!(*state, b"pass" | b"fail" | b"pending" | b"skipping" | b"cancel")
                && !duration.is_empty()
                && (url.is_empty() || url.starts_with(b"http://") || url.starts_with(b"https://")))
        {
            return None;
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
        return None;
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
    Some(output)
}

fn compact_gh_run_list(stdout: &[u8]) -> Option<Vec<u8>> {
    const HEADER: &[u8] = b"STATUS TITLE WORKFLOW BRANCH EVENT ID ELAPSED AGE";
    let table = compact_gh_table(stdout)?;
    let mut lines = table
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty());
    let header = lines.next()?;
    let header = collapse_table(header);
    if header.strip_suffix(b"\n") != Some(HEADER) {
        return None;
    }
    let mut rows = 0usize;
    for line in lines {
        let fields = line.split(|byte| *byte == b'\t').collect::<Vec<_>>();
        if fields.len() != 8
            || fields.iter().any(|field| field.is_empty())
            || !matches!(
                fields[0],
                b"completed" | b"in_progress" | b"queued" | b"failure" | b"cancelled"
            )
            || !fields[5].iter().all(u8::is_ascii_digit)
        {
            return None;
        }
        rows += 1;
    }
    (rows > 0).then(|| collapse_table(&table))
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
use super::table::collapse_table;
use super::{append_line, find_subslice};
