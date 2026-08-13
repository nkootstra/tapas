pub(crate) fn flag_on(name: &str) -> bool {
    std::env::var_os(name).and_then(|value| value.as_encoded_bytes().first().copied()) == Some(b'1')
}

pub(crate) fn flag_off(name: &str) -> bool {
    std::env::var_os(name)
        .is_some_and(|value| matches!(value.as_encoded_bytes(), b"0" | b"false" | b"no" | b"off"))
}
