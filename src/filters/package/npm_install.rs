use super::*;

pub(super) fn compact_npm_install(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if (looks_like_pnpm(stdout) || looks_like_pnpm(stderr))
        && let Some(output) = compact_pnpm(stdout, stderr)
    {
        return output;
    }
    if looks_like_npm(stdout) || looks_like_npm(stderr) {
        return compact_npm(stdout, stderr);
    }
    if (looks_like_bun_yarn(stdout) || looks_like_bun_yarn(stderr))
        && let Some(output) = compact_bun_yarn(stdout, stderr)
    {
        return output;
    }
    let mut output = Vec::new();
    let mut kept_lines = 0usize;
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = trim_ascii(&clean);
            if should_keep_install_line(line) {
                output.extend_from_slice(line);
                output.push(b'\n');
                kept_lines += 1;
            }
        }
    }
    if output.is_empty() {
        output.extend_from_slice(b"up to date\n");
    }
    head_tail(output, kept_lines, 40, 20)
}

fn looks_like_pnpm(input: &[u8]) -> bool {
    [
        b"Packages: +".as_slice(),
        b"Packages: -",
        b"Progress: ",
        b"Lockfile is up to date",
        b"\ndependencies:\n",
        b"\ndevDependencies:\n",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}
