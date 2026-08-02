use super::*;

pub(super) fn requests_exact_output(argv: &[&[u8]]) -> bool {
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version" | b"-h" | b"-V") {
            return true;
        }
        if is_dotnet_query_switch(argument) {
            return true;
        }
    }
    matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
}

fn is_dotnet_query_switch(argument: &[u8]) -> bool {
    let rest = if let Some(rest) = argument.strip_prefix(b"--") {
        rest
    } else if matches!(argument.first(), Some(b'-' | b'/')) {
        &argument[1..]
    } else {
        return false;
    };
    [b"getproperty".as_slice(), b"getitem", b"gettargetresult"]
        .iter()
        .any(|query| {
            rest.get(..query.len())
                .is_some_and(|name| name.eq_ignore_ascii_case(query))
                && rest.get(query.len()) == Some(&b':')
        })
}

pub(super) fn trim_ascii_start(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[1..];
    }
    input
}

pub(super) fn trim_ascii(input: &[u8]) -> &[u8] {
    trim_ascii_end(trim_ascii_start(input))
}

pub(super) fn strip_ansi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        if input[index] != 0x1b {
            output.push(input[index]);
            index += 1;
            continue;
        }
        match input.get(index + 1) {
            Some(b'[') => {
                index += 2;
                while index < input.len() && !(0x40..=0x7e).contains(&input[index]) {
                    index += 1;
                }
                index += usize::from(index < input.len());
            }
            Some(b']') => {
                index += 2;
                while index < input.len() {
                    if input[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    output
}
