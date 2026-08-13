pub(super) fn compact_curl_trace(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut in_certificate = false;
    let mut in_server_certificate = false;
    let mut requests = 0usize;

    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        if find_subslice(line, b"-----BEGIN CERTIFICATE-----").is_some() {
            in_certificate = true;
            continue;
        }
        if in_certificate {
            if find_subslice(line, b"-----END CERTIFICATE-----").is_some() {
                in_certificate = false;
            }
            continue;
        }
        if line.starts_with(b"* Server certificate:") {
            in_server_certificate = true;
            continue;
        }
        if in_server_certificate {
            if line.first() == Some(&b'*') {
                continue;
            }
            in_server_certificate = false;
        }

        match line[0] {
            b'>' => {
                if is_curl_request_line(line) {
                    requests += 1;
                    if requests <= 5 {
                        append_line(&mut output, line);
                    }
                } else if requests <= 1 {
                    append_line(&mut output, line);
                }
            }
            b'<' => {
                if line.starts_with(b"< HTTP/") {
                    if requests <= 5 {
                        append_line(&mut output, line);
                    }
                } else if requests <= 1
                    || line.starts_with(b"< location:")
                    || line.starts_with(b"< Location:")
                {
                    append_line(&mut output, line);
                }
            }
            b'*' => {
                if !drop_curl_meta(line) && keep_curl_meta(line) && requests <= 1 {
                    append_line(&mut output, line);
                }
            }
            _ => append_line(&mut output, line),
        }
    }
    if requests > 1 {
        output.push(b'(');
        output.extend_from_slice(requests.to_string().as_bytes());
        output.extend_from_slice(b" requests total)\n");
    }
    output
}

fn is_curl_request_line(line: &[u8]) -> bool {
    let Some(after) = line.strip_prefix(b"> ") else {
        return false;
    };
    let Some(space) = after.iter().position(|byte| *byte == b' ') else {
        return false;
    };
    space > 0 && after[..space].iter().all(u8::is_ascii_uppercase)
}

fn keep_curl_meta(line: &[u8]) -> bool {
    [
        b"* Connected to".as_slice(),
        b"* Trying",
        b"* Closing",
        b"* Host:",
        b"* Request completely sent",
        b"* HTTP/",
        b"* Mark bundle",
        b"* schannel:",
        b"* Rebuilt URL to:",
        b"* Re-using existing connection",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn drop_curl_meta(line: &[u8]) -> bool {
    [
        b"* TLSv".as_slice(),
        b"* SSL connection",
        b"* ALPN",
        b"* Server certificate:",
        b"*   subject:",
        b"*   issuer:",
        b"*   SSL certificate verify",
        b"*   start date:",
        b"*   expire date:",
        b"*   common name:",
        b"*   subjectAltName:",
        b"*   using ",
        b"* Server auth using",
        b"* Using HTTP",
        b"* schannel: encrypted data",
        b"* schannel: decrypted data",
        b"*  CAfile:",
        b"*  CApath:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

pub(super) fn compact_curl(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut method = b"".as_slice();
    let mut path = b"".as_slice();
    let mut last_path = b"".as_slice();
    let mut host = b"".as_slice();
    let mut status = b"".as_slice();
    let mut content_type = b"".as_slice();
    let mut content_length = b"".as_slice();
    let mut request_count = 0usize;
    let mut status_count = 0usize;
    let mut same_status = true;
    let mut location_count = 0usize;
    for raw in stderr.split(|byte| *byte == b'\n') {
        let line = raw.trim_ascii();
        if let Some(request) = line.strip_prefix(b"> ") {
            let fields = request
                .trim_ascii()
                .split(|byte| *byte == b' ')
                .collect::<Vec<_>>();
            if fields.len() >= 3
                && fields[0].iter().all(u8::is_ascii_uppercase)
                && fields[2].starts_with(b"HTTP/")
            {
                request_count += 1;
                if request_count == 1 {
                    method = fields[0];
                    path = fields[1];
                }
                last_path = fields[1];
            }
        }
        if host.is_empty()
            && let Some(value) = strip_prefix_ignore_ascii_case(line, b"> Host:")
        {
            host = value.trim_ascii();
        }
        if line.starts_with(b"< HTTP/") {
            status_count += 1;
            let current = line[2..].trim_ascii();
            if status_count == 1 {
                status = current;
            } else if current != status {
                same_status = false;
            }
        }
        if strip_prefix_ignore_ascii_case(line, b"< location:").is_some() {
            location_count += 1;
        }
        if content_type.is_empty()
            && let Some(value) = strip_prefix_ignore_ascii_case(line, b"< content-type:")
        {
            content_type = value
                .trim_ascii()
                .split(|byte| *byte == b';')
                .next()
                .unwrap_or_default();
        }
        if content_length.is_empty()
            && let Some(value) = strip_prefix_ignore_ascii_case(line, b"< content-length:")
        {
            content_length = value.trim_ascii();
        }
    }
    let can_summarize = request_count > 0
        && status_count > 0
        && !status.is_empty()
        && location_count == 0
        && (request_count != 1 || status_count == 1)
        && (request_count <= 1 || status_count == request_count && same_status);
    if !can_summarize {
        let mut output = Vec::with_capacity(stdout.len() + stderr.len());
        if !stdout.is_empty() {
            output.extend_from_slice(b"--- headers ---\n");
        }
        output.extend_from_slice(&compact_curl_trace(stderr));
        if !stdout.is_empty() {
            output.extend_from_slice(b"--- body ---\n");
            output.extend_from_slice(stdout);
        }
        return output;
    }

    let mut output = b"curl ".to_vec();
    if request_count > 1 {
        output.extend_from_slice(request_count.to_string().as_bytes());
        output.push(b' ');
    }
    output.extend_from_slice(method);
    output.push(b' ');
    output.extend_from_slice(host);
    if !path.is_empty() {
        if !host.is_empty() && !path.starts_with(b"/") {
            output.push(b' ');
        }
        output.extend_from_slice(path);
    }
    if request_count > 1 && last_path != path {
        output.extend_from_slice(b"..");
        output.extend_from_slice(last_path);
    }
    output.extend_from_slice(b" -> ");
    output.extend_from_slice(status);
    if request_count > 1 {
        output.extend_from_slice(b" x");
        output.extend_from_slice(request_count.to_string().as_bytes());
    }
    if !content_type.is_empty() {
        output.push(b' ');
        output.extend_from_slice(content_type);
    }
    if request_count == 1 && !content_length.is_empty() {
        output.extend_from_slice(b" len=");
        output.extend_from_slice(content_length);
    }
    output.push(b'\n');
    output.extend_from_slice(stdout);
    output
}

pub(super) fn is_single_verbose_invocation(argv: &[&[u8]]) -> bool {
    let mut verbosity = 0usize;
    for argument in argv[1..].iter().take_while(|argument| **argument != b"--") {
        if *argument == b"--verbose" {
            verbosity += 1;
        } else if argument.starts_with(b"--") {
            if *argument == b"--no-verbose" {
                return false;
            }
        } else if let Some(options) = argument.strip_prefix(b"-") {
            verbosity += options.iter().filter(|option| **option == b'v').count();
        }
    }
    verbosity == 1
}

pub(super) fn matches_classic_verbose_trace(input: &[u8]) -> bool {
    if input.is_empty() || std::str::from_utf8(input).is_err() {
        return false;
    }
    let mut requests = 0usize;
    let mut statuses = 0usize;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        match line[0] {
            b'*' => {}
            b'>' => requests += usize::from(is_curl_request_line(line)),
            b'<' => statuses += usize::from(line.starts_with(b"< HTTP/")),
            _ => return false,
        }
    }
    requests > 0 && statuses > 0 && statuses >= requests
}
use super::table::strip_prefix_ignore_ascii_case;
use super::{append_line, find_subslice};
