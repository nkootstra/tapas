use super::exact::trim_ascii;

#[derive(Clone, Copy)]
enum Route {
    Tasks,
    Describe,
    Prereqs,
    Where,
}

pub(super) fn is_metadata_route(argv: &[&[u8]]) -> bool {
    route(argv).is_some()
}

pub(super) fn matches(argv: &[&[u8]], input: &[u8]) -> bool {
    let Some(route) = route(argv) else {
        return false;
    };
    input.split(|byte| *byte == b'\n').any(|raw| {
        let line = trim_ascii(raw);
        match route {
            Route::Tasks => {
                line.starts_with(b"rake ") && line.windows(3).any(|part| part == b" # ")
            }
            Route::Describe => line.starts_with(b"rake "),
            Route::Prereqs => line.starts_with(b"rake ") || raw.starts_with(b"    "),
            Route::Where => {
                line.starts_with(b"rake ")
                    && (line.windows(2).any(|part| part == b" /")
                        || line.windows(3).any(|part| part == b" # "))
            }
        }
    })
}

fn route(argv: &[&[u8]]) -> Option<Route> {
    let arguments = argv.get(1..)?;
    let (route, allows_pattern) = match arguments.first().copied()? {
        b"-T" | b"--tasks" => (Route::Tasks, true),
        b"-D" | b"--describe" => (Route::Describe, true),
        b"-P" | b"--prereqs" => (Route::Prereqs, true),
        b"-W" | b"--where" => (Route::Where, true),
        _ => return None,
    };
    if arguments.len() == 1
        || allows_pattern
            && arguments.len() == 2
            && !arguments[1].is_empty()
            && !arguments[1].starts_with(b"-")
    {
        Some(route)
    } else {
        None
    }
}
