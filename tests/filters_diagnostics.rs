use tapas::filters::{EvidenceClass, StreamFilterOutput, diagnostics};
mod common;
use common::fixture;

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
        "    x: int = \"foo\"\n",
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
fn ruff_and_mypy_require_recognizable_human_output() {
    for (argv, output) in [
        (
            &[b"ruff".as_slice(), b"check", b"."][..],
            br#"{"diagnostics":[{"filename":"src/app.py","location":{"row":1}}]}"#.as_slice(),
        ),
        (
            &[b"mypy".as_slice(), b"src"][..],
            b"cache_dir = .mypy_cache\nplugins = custom.plugin\n".as_slice(),
        ),
    ] {
        let filtered = diagnostics::dispatch_streams_argv(argv, output, b"", 1, false).unwrap();
        assert_eq!(filtered.stdout, output);
        assert_eq!(filtered.evidence, EvidenceClass::ByteExact);
    }

    let non_utf8 = b"src/app.py:1:1: E999 invalid \xff\n";
    let filtered =
        diagnostics::dispatch_streams_argv(&[b"ruff", b"check", b"."], non_utf8, b"", 1, false)
            .unwrap();
    assert_eq!(filtered.stdout, non_utf8);
    assert_eq!(filtered.evidence, EvidenceClass::ByteExact);
}

#[test]
fn ruff_preserves_full_human_diagnostic_context() {
    let input = concat!(
        "F401 [*] `os` imported but unused\n",
        " --> src/app.py:1:8\n",
        "  |\n",
        "1 | import os\n",
        "  |        ^^\n",
        "  |\n",
        "help: Remove unused import: `os`\n",
        "\n",
        "Found 1 error.\n",
    );
    let output = diagnostics::dispatch_streams_argv(
        &[b"ruff", b"check", b"."],
        input.as_bytes(),
        b"",
        1,
        false,
    )
    .unwrap();
    assert_eq!(output.stdout, input.as_bytes());
    assert_eq!(output.evidence, EvidenceClass::FactComplete);
}

#[test]
fn golangci_lint_run_compacts_human_diagnostics_per_stream() {
    let stdout = concat!(
        "level=info msg=\"starting linters\"\n",
        "pkg/a.go:12:4: shadow: declaration of \"err\" shadows declaration (govet)\n",
        "\tif err := work(); err != nil {\n",
        "\t   ^\n",
        "2 issues:\n",
    );
    let stderr = b"warning: cache directory unavailable\n";
    let output = diagnostics::dispatch_streams_argv(
        &[b"golangci-lint", b"run"],
        stdout.as_bytes(),
        stderr,
        1,
        false,
    )
    .unwrap();
    assert_eq!(
        output.stdout,
        concat!(
            "pkg/a.go:12:4: shadow: declaration of \"err\" shadows declaration (govet)\n",
            "\tif err := work(); err != nil {\n",
            "\t   ^\n",
            "2 issues:\n",
        )
        .as_bytes()
    );
    assert_eq!(output.stderr, stderr);
    assert_eq!(output.evidence, EvidenceClass::FactComplete);
}

#[test]
fn golangci_lint_run_supports_clean_and_rejects_unknown_failure_output() {
    let clean = diagnostics::dispatch_streams_argv(&[b"golangci-lint", b"run"], b"", b"", 0, false)
        .unwrap();
    assert_eq!(clean.evidence, EvidenceClass::FactComplete);

    let failure = b"level=error msg=\"failed to load config\"\n";
    let unknown =
        diagnostics::dispatch_streams_argv(&[b"golangci-lint", b"run"], b"", failure, 3, false)
            .unwrap();
    assert_eq!(unknown.stderr, failure);
    assert_eq!(unknown.evidence, EvidenceClass::ByteExact);
}

#[test]
fn rubocop_preserves_offense_source_caret_and_summary() {
    let input = concat!(
        "Inspecting 1 file\n",
        "C\n",
        "\n",
        "Offenses:\n",
        "\n",
        "app/models/user.rb:3:7: C: [Correctable] Style/StringLiterals: Prefer single-quoted strings.\n",
        "puts \"hello\"\n",
        "     ^^^^^^^\n",
        "\n",
        "1 file inspected, 1 offense detected, 1 offense autocorrectable\n",
    );
    let output =
        diagnostics::dispatch_streams_argv(&[b"rubocop"], b"", input.as_bytes(), 1, false).unwrap();
    assert_eq!(
        output.stderr,
        concat!(
            "app/models/user.rb:3:7: C: [Correctable] Style/StringLiterals: Prefer single-quoted strings.\n",
            "puts \"hello\"\n",
            "     ^^^^^^^\n",
            "1 file inspected, 1 offense detected, 1 offense autocorrectable\n",
        )
        .as_bytes()
    );
    assert_eq!(output.evidence, EvidenceClass::FactComplete);
}

#[test]
fn rubocop_clean_summary_is_fact_complete() {
    let input = b"Inspecting 2 files\n..\n\n2 files inspected, no offenses detected\n";
    let output = diagnostics::dispatch_streams_argv(&[b"rubocop"], input, b"", 0, false).unwrap();
    assert_eq!(output.stdout, b"2 files inspected, no offenses detected\n");
    assert_eq!(output.evidence, EvidenceClass::FactComplete);
}

#[test]
fn dedicated_diagnostics_routes_passthrough_structured_and_unknown_shapes() {
    let cases = [
        (
            &[b"golangci-lint".as_slice(), b"run"][..],
            br#"{"Issues":[{"FromLinter":"govet","Text":"shadow"}]}"#.as_slice(),
        ),
        (
            &[b"golangci-lint".as_slice(), b"run"][..],
            b"<?xml version=\"1.0\"?><checkstyle></checkstyle>\n".as_slice(),
        ),
        (
            &[b"rubocop".as_slice()][..],
            b"::error file=user.rb,line=3,col=7::Style/StringLiterals\n".as_slice(),
        ),
        (
            &[b"rubocop".as_slice()][..],
            b"<!doctype html><html><body>report</body></html>\n".as_slice(),
        ),
    ];
    for (argv, input) in cases {
        let output = diagnostics::dispatch_streams_argv(argv, input, b"", 1, false).unwrap();
        assert_eq!(output.stdout, input);
        assert_eq!(output.evidence, EvidenceClass::ByteExact);
    }
}

#[test]
fn dedicated_diagnostics_preserve_windows_drive_locations() {
    let golangci = b"C:\\repo\\pkg\\a.go:12:4: shadowed variable (govet)\n";
    let golangci_output =
        diagnostics::dispatch_streams_argv(&[b"golangci-lint", b"run"], golangci, b"", 1, false)
            .unwrap();
    assert_eq!(golangci_output.stdout, golangci);
    assert_eq!(golangci_output.evidence, EvidenceClass::FactComplete);

    let rubocop = concat!(
        "C:\\repo\\user.rb:3:7: C: Style/StringLiterals: Prefer single quotes.\n",
        "puts \"hello\"\n",
        "     ^^^^^^^\n",
        "1 file inspected, 1 offense detected\n",
    );
    let rubocop_output =
        diagnostics::dispatch_streams_argv(&[b"rubocop"], rubocop.as_bytes(), b"", 1, false)
            .unwrap();
    assert_eq!(rubocop_output.stdout, rubocop.as_bytes());
    assert_eq!(rubocop_output.evidence, EvidenceClass::FactComplete);
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
    let input = fixture("pre_commit_failed.txt");
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
    assert!(check.stdout.is_empty());
    assert!(contains(&check.stderr, b"[warn] needsfix.ts"));
    assert!(contains(&check.stderr, b"Code style issues found"));
    assert!(!check.stderr.contains(&0x1b));

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
