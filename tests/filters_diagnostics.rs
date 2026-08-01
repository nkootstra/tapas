use tapas::filters::{EvidenceClass, StreamFilterOutput, diagnostics};

const FIXTURES: &str = "compat/smll-v1.9.0/fixtures/tests/fixtures";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/{FIXTURES}/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn benchmark_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/compat/smll-v1.9.0/fixtures/benchmarks/smll-vs-rtk/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn mypy_keeps_diagnostics_summaries_and_pretty_carets() {
    let input = concat!(
        "LOG: processing\n",
        "\x1b[31msrc/a.py:10:5: error: Incompatible types [assignment]\x1b[0m\n",
        "    x: int = \"foo\"\n",
        "             ^~~~~\n",
        "Found 1 error in 1 file\n",
    );
    let expected = concat!(
        "src/a.py:10:5: error: Incompatible types [assignment]\n",
        "             ^~~~~\n",
        "Found 1 error in 1 file\n",
    );
    assert_eq!(
        diagnostics::dispatch_streams_argv(&[b"mypy", b"src"], input.as_bytes(), b"", 1, false,)
            .unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn ruff_groups_diagnostics_by_path_without_losing_locations() {
    let input = concat!(
        "src/app.py:1:8: F401 `os` imported but unused\n",
        "src/app.py:9:1: E501 line too long\n",
        "src/util.py:3:5: F841 local variable assigned but never used\n",
        "Found 3 errors.\n",
    );
    let expected = concat!(
        "src/app.py\n",
        "  1:8 F401 `os` imported but unused\n",
        "  9:1 E501 line too long\n",
        "src/util.py\n",
        "  3:5 F841 local variable assigned but never used\n",
        "Found 3 errors.\n",
    );
    assert_eq!(
        diagnostics::dispatch_streams_argv(
            &[b"ruff", b"check", b"."],
            input.as_bytes(),
            b"",
            1,
            false,
        )
        .unwrap()
        .stdout,
        expected.as_bytes(),
    );
}

#[test]
fn eslint_stylish_keeps_file_diagnostics_and_summary() {
    let input = concat!(
        "ESLint is running\n\n",
        "/repo/src/app.ts\n",
        "  1:7   error    'unused' is assigned a value but never used  no-unused-vars\n",
        "  2:10  warning  Unexpected console statement                no-console\n",
        "\n✖ 2 problems (1 error, 1 warning)\n",
    );
    let output =
        diagnostics::dispatch_streams_argv(&[b"eslint", b"."], input.as_bytes(), b"", 1, false)
            .unwrap();
    assert_eq!(output.evidence, EvidenceClass::FactComplete);
    assert_eq!(
        output.stdout,
        concat!(
            "/repo/src/app.ts\n",
            "1:7 error 'unused' is assigned a value but never used no-unused-vars\n",
            "2:10 warning Unexpected console statement no-console\n",
            "✖ 2 problems (1 error, 1 warning)\n",
        )
        .as_bytes(),
    );
}

#[test]
fn precommit_fixture_keeps_failed_hook_and_counts_passes() {
    let input = benchmark_fixture("pre_commit_failed.txt");
    let output = diagnostics::dispatch_streams_argv(
        &[b"pre-commit", b"run", b"--all-files"],
        &input,
        b"",
        1,
        false,
    )
    .unwrap();
    assert_eq!(output.evidence, EvidenceClass::FactComplete);
    assert!(contains(&output.stdout, b"Check Yaml Failed"));
    assert!(contains(&output.stdout, b"- hook id: check-yaml"));
    assert!(contains(&output.stdout, b"- exit code: 1"));
    assert!(contains(&output.stdout, b"passed:"));
    assert!(!contains(&output.stdout, b"Installing environment"));
}

#[test]
fn prettier_colored_check_and_write_modes_match_compact_contract() {
    let colored = fixture("prettier_check_color.txt");
    let check = diagnostics::dispatch_streams_argv(
        &[b"prettier", b"--check", b"."],
        b"",
        &colored,
        1,
        false,
    )
    .unwrap();
    assert!(contains(&check.stdout, b"[warn] needsfix.ts"));
    assert!(contains(&check.stdout, b"Code style issues found"));
    assert!(!check.stdout.contains(&0x1b));

    let write = fixture("prettier_write.txt");
    let output =
        diagnostics::dispatch_streams_argv(&[b"prettier", b"--write", b"."], &write, b"", 0, false)
            .unwrap();
    assert!(contains(&output.stdout, b"formatted 10:"));
    assert!(contains(&output.stdout, b"(+2)"));
    assert!(!contains(&output.stdout, b"26ms"));
}

#[test]
fn tofu_plan_fixture_keeps_actionable_diffs_and_drops_refresh_chatter() {
    let input = fixture("tofu_plan_update.txt");
    let output =
        diagnostics::dispatch_streams_argv(&[b"tofu", b"plan"], &input, b"", 0, false).unwrap();
    assert!(contains(
        &output.stdout,
        b"~ input  = \"small\" -> \"large\""
    ));
    assert!(contains(&output.stdout, b"# forces replacement"));
    assert!(contains(
        &output.stdout,
        b"Plan: 1 to add, 1 to change, 1 to destroy."
    ));
    assert!(!contains(&output.stdout, b"Refreshing state"));
    assert!(!contains(&output.stdout, b"update in-place (current"));
}

#[test]
fn lossless_query_unknown_and_invalid_inputs_fail_open() {
    let stdout = b"raw \xff\n";
    let stderr = b"err \xfe\n";
    for argv in [
        &[b"ruff".as_slice(), b"--help"][..],
        &[b"terraform".as_slice(), b"show", b"--json"][..],
        &[b"unknown".as_slice()][..],
    ] {
        assert_eq!(
            diagnostics::dispatch_streams_argv(argv, stdout, stderr, 0, false).unwrap(),
            StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact,),
        );
    }
    assert_eq!(
        diagnostics::dispatch_streams_argv(&[b"mypy"], stdout, stderr, 1, true).unwrap(),
        StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact,),
    );
    assert!(diagnostics::dispatch_streams_argv(&[], stdout, stderr, 0, false).is_err());
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
