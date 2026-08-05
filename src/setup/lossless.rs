use std::ops::Range;

use super::{Target, Value, json};

pub(super) enum RemoveResult {
    Removed(Vec<u8>),
    Missing,
    Duplicate,
}

pub(super) fn add_hook(existing: Option<&[u8]>, entry: &Value) -> Result<(Vec<u8>, bool), ()> {
    let Some(input) = existing else {
        let mut output = b"{\"hooks\":{\"PreToolUse\":[".to_vec();
        output.extend_from_slice(&json::serialize(entry));
        output.extend_from_slice(b"]}}\n");
        return Ok((output, false));
    };
    validate_document(input)?;
    let root = root_range(input)?;
    let root_object = object_info(input, root)?;
    let Some(hooks) = object_member(&root_object, b"hooks")? else {
        let addition = member_bytes(b"hooks", &pre_tool_use_object(entry));
        return Ok((insert_member(input, &root_object, &addition), false));
    };
    let hooks_object = object_info(input, hooks)?;
    let Some(events) = object_member(&hooks_object, b"PreToolUse")? else {
        let addition = member_bytes(b"PreToolUse", &Value::Array(vec![entry.clone()]));
        return Ok((insert_member(input, &hooks_object, &addition), false));
    };
    let elements = array_elements(input, events.clone())?;
    let matches = elements
        .iter()
        .filter(|range| json::parse(&input[(**range).clone()]).ok().as_ref() == Some(entry))
        .count();
    match matches {
        0 => {
            let close = trim_end_ws(input, events.start + 1, events.end - 1);
            let mut insertion = Vec::new();
            if !elements.is_empty() {
                insertion.push(b',');
            }
            insertion.extend_from_slice(&json::serialize(entry));
            Ok((splice(input, close, close, &insertion), false))
        }
        1 => Ok((input.to_vec(), true)),
        _ => Err(()),
    }
}

pub(super) fn remove_hook(input: &[u8], entry: &Value) -> Result<RemoveResult, ()> {
    validate_document(input)?;
    let root = object_info(input, root_range(input)?)?;
    let Some(hooks) = object_member(&root, b"hooks")? else {
        return Ok(RemoveResult::Missing);
    };
    let hooks = object_info(input, hooks)?;
    let Some(events) = object_member(&hooks, b"PreToolUse")? else {
        return Ok(RemoveResult::Missing);
    };
    let elements = array_elements(input, events)?;
    let matching: Vec<usize> = elements
        .iter()
        .enumerate()
        .filter_map(|(index, range)| {
            (json::parse(&input[range.clone()]).ok().as_ref() == Some(entry)).then_some(index)
        })
        .collect();
    if matching.len() > 1 {
        return Ok(RemoveResult::Duplicate);
    }
    let Some(index) = matching.first().copied() else {
        return Ok(RemoveResult::Missing);
    };
    let range = array_element_removal_range(&elements, index);
    Ok(RemoveResult::Removed(splice(
        input,
        range.start,
        range.end,
        b"",
    )))
}

pub(super) fn tapas_hook_count(input: &[u8], target: Target) -> Result<usize, ()> {
    validate_document(input)?;
    let root = object_info(input, root_range(input)?)?;
    let Some(hooks) = object_member(&root, b"hooks")? else {
        return Ok(0);
    };
    let hooks = object_info(input, hooks)?;
    let Some(events) = object_member(&hooks, b"PreToolUse")? else {
        return Ok(0);
    };
    let mut count = 0;
    for element in array_elements(input, events)? {
        let value = json::parse(&input[element]).map_err(|_| ())?;
        let Some(Value::Array(handlers)) = value.get(b"hooks") else {
            continue;
        };
        for handler in handlers {
            let Some(Value::String(command)) = handler.get(b"command") else {
                continue;
            };
            let mut marker = b"--hook-eval ".to_vec();
            marker.extend_from_slice(target.name().as_bytes());
            if crate::filters::find_subslice(command, &marker).is_some() {
                count += 1;
            }
        }
    }
    Ok(count)
}

pub(super) fn remove_root_array_strings(
    input: &[u8],
    key: &[u8],
    values: &[Vec<u8>],
) -> Result<(Vec<u8>, usize), ()> {
    let mut output = input.to_vec();
    let mut removed = 0;
    loop {
        validate_document(&output)?;
        let root = object_info(&output, root_range(&output)?)?;
        let Some(array) = object_member(&root, key)? else {
            return Ok((output, removed));
        };
        let elements = array_elements(&output, array)?;
        let matching = elements.iter().position(|range| {
            matches!(json::parse(&output[range.clone()]), Ok(Value::String(value)) if values.contains(&value))
        });
        let Some(index) = matching else {
            return Ok((output, removed));
        };
        let range = array_element_removal_range(&elements, index);
        output = splice(&output, range.start, range.end, b"");
        removed += 1;
    }
}

fn array_element_removal_range(elements: &[Range<usize>], index: usize) -> Range<usize> {
    if elements.len() == 1 {
        elements[index].clone()
    } else if index + 1 < elements.len() {
        elements[index].start..elements[index + 1].start
    } else {
        elements[index - 1].end..elements[index].end
    }
}

fn validate_document(input: &[u8]) -> Result<(), ()> {
    if input.is_empty()
        || input.starts_with(&[0xef, 0xbb, 0xbf])
        || std::str::from_utf8(input).is_err()
        || input.iter().all(u8::is_ascii_whitespace)
        || !matches!(json::parse(input), Ok(Value::Object(_)))
    {
        return Err(());
    }
    Ok(())
}

fn pre_tool_use_object(entry: &Value) -> Value {
    Value::Object(vec![(
        b"PreToolUse".to_vec(),
        Value::Array(vec![entry.clone()]),
    )])
}

fn member_bytes(key: &[u8], value: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    json::write_string(key, &mut bytes);
    bytes.push(b':');
    bytes.extend_from_slice(&json::serialize(value));
    bytes
}

#[derive(Clone)]
struct ObjectInfo {
    range: Range<usize>,
    members: Vec<(Vec<u8>, Range<usize>)>,
}

fn root_range(input: &[u8]) -> Result<Range<usize>, ()> {
    let start = skip_ws(input, 0);
    let end = value_end(input, start)?;
    (skip_ws(input, end) == input.len())
        .then_some(start..end)
        .ok_or(())
}

fn object_info(input: &[u8], range: Range<usize>) -> Result<ObjectInfo, ()> {
    if input.get(range.start) != Some(&b'{') || input.get(range.end - 1) != Some(&b'}') {
        return Err(());
    }
    let mut position = skip_ws(input, range.start + 1);
    let mut members = Vec::new();
    if position == range.end - 1 {
        return Ok(ObjectInfo { range, members });
    }
    loop {
        let key_start = position;
        let key_end = string_end(input, key_start)?;
        let key = match json::parse(&input[key_start..key_end]).map_err(|_| ())? {
            Value::String(value) => value,
            _ => return Err(()),
        };
        position = skip_ws(input, key_end);
        if input.get(position) != Some(&b':') {
            return Err(());
        }
        let value_start = skip_ws(input, position + 1);
        let value_end = value_end(input, value_start)?;
        members.push((key, value_start..value_end));
        position = skip_ws(input, value_end);
        match input.get(position) {
            Some(b'}') if position == range.end - 1 => break,
            Some(b',') => position = skip_ws(input, position + 1),
            _ => return Err(()),
        }
    }
    Ok(ObjectInfo { range, members })
}

fn object_member(object: &ObjectInfo, key: &[u8]) -> Result<Option<Range<usize>>, ()> {
    let matching: Vec<_> = object
        .members
        .iter()
        .filter(|(candidate, _)| candidate == key)
        .collect();
    match matching.as_slice() {
        [] => Ok(None),
        [(_, range)] => Ok(Some(range.clone())),
        _ => Err(()),
    }
}

fn insert_member(input: &[u8], object: &ObjectInfo, addition: &[u8]) -> Vec<u8> {
    let close = trim_end_ws(input, object.range.start + 1, object.range.end - 1);
    let mut insertion = Vec::new();
    if !object.members.is_empty() {
        insertion.push(b',');
    }
    insertion.extend_from_slice(addition);
    splice(input, close, close, &insertion)
}

fn array_elements(input: &[u8], range: Range<usize>) -> Result<Vec<Range<usize>>, ()> {
    if input.get(range.start) != Some(&b'[') || input.get(range.end - 1) != Some(&b']') {
        return Err(());
    }
    let mut position = skip_ws(input, range.start + 1);
    let mut elements = Vec::new();
    if position == range.end - 1 {
        return Ok(elements);
    }
    loop {
        let end = value_end(input, position)?;
        elements.push(position..end);
        position = skip_ws(input, end);
        match input.get(position) {
            Some(b']') if position == range.end - 1 => break,
            Some(b',') => position = skip_ws(input, position + 1),
            _ => return Err(()),
        }
    }
    Ok(elements)
}

fn value_end(input: &[u8], start: usize) -> Result<usize, ()> {
    match input.get(start).copied().ok_or(())? {
        b'"' => string_end(input, start),
        b'{' | b'[' => composite_end(input, start),
        _ => {
            let mut end = start;
            while input
                .get(end)
                .is_some_and(|byte| !byte.is_ascii_whitespace() && !b",]}".contains(byte))
            {
                end += 1;
            }
            (end > start).then_some(end).ok_or(())
        }
    }
}

fn composite_end(input: &[u8], start: usize) -> Result<usize, ()> {
    let mut stack = vec![input[start]];
    let mut position = start + 1;
    while let Some(byte) = input.get(position).copied() {
        match byte {
            b'"' => position = string_end(input, position)?,
            b'{' | b'[' => {
                stack.push(byte);
                position += 1;
            }
            b'}' if stack.pop() == Some(b'{') => {
                position += 1;
                if stack.is_empty() {
                    return Ok(position);
                }
            }
            b']' if stack.pop() == Some(b'[') => {
                position += 1;
                if stack.is_empty() {
                    return Ok(position);
                }
            }
            b'}' | b']' => return Err(()),
            _ => position += 1,
        }
    }
    Err(())
}

fn string_end(input: &[u8], start: usize) -> Result<usize, ()> {
    if input.get(start) != Some(&b'"') {
        return Err(());
    }
    let mut position = start + 1;
    while let Some(byte) = input.get(position).copied() {
        match byte {
            b'"' => return Ok(position + 1),
            b'\\' => position += 2,
            _ => position += 1,
        }
    }
    Err(())
}

fn skip_ws(input: &[u8], mut position: usize) -> usize {
    while input.get(position).is_some_and(u8::is_ascii_whitespace) {
        position += 1;
    }
    position
}

fn trim_end_ws(input: &[u8], start: usize, mut end: usize) -> usize {
    while end > start && input[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

fn splice(input: &[u8], start: usize, end: usize, replacement: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() - (end - start) + replacement.len());
    output.extend_from_slice(&input[..start]);
    output.extend_from_slice(replacement);
    output.extend_from_slice(&input[end..]);
    output
}
