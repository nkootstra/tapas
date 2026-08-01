pub(crate) fn flag_on(name: &str) -> bool {
    std::env::var_os(name).and_then(|value| value.as_encoded_bytes().first().copied()) == Some(b'1')
}
