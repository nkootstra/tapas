use tapas::filters::{EvidenceClass, FilterOutput, test_tools};
mod common;
use common::fixture;

#[test]
fn cargo_test_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("cargo_test_failing.txt");
    let expected = concat!(
        "test tests::div_by_zero ... FAILED\n",
        "test tests::sub ... ok\n",
        "test tests::edge_overflow ... FAILED\n",
        "---- tests::div_by_zero stdout ----\n",
        "thread 'tests::div_by_zero' panicked at src/lib.rs:42:9:\n",
        "attempt to divide by zero\n",
        "---- tests::edge_overflow stdout ----\n",
        "thread 'tests::edge_overflow' panicked at src/lib.rs:58:13:\n",
        "assertion `left == right` failed\n",
        "left: 2147483647\n",
        "right: -2147483648\n",
        "res 3p 2f 0.01s\n",
    );

    assert!(test_tools::matches(&input));
    assert_eq!(
        test_tools::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn jest_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("jest_failing.txt");
    let expected = concat!(
        "FAIL  src/components/Button.test.tsx\n",
        "Button component\n",
        "✗ renders with uppercase label (12 ms)\n",
        "● Button component › renders with uppercase label\n",
        "expect(received).toBe(expected) // Object.is equality\n",
        "Expected: \"Submit\"\n",
        "Received: \"submit\"\n",
        "> 14 |     expect(screen.getByText('Submit')).toBeInTheDocument();\n",
        "|            ^\n",
        "15 |   });\n",
        "16 | });\n",
        "FAIL  src/api/client.test.ts\n",
        "client\n",
        "✗ handles 401 (88 ms)\n",
        "● client › handles 401\n",
        "Error: Network timeout after 5000ms\n",
        "Test Suites: 2 failed, 3 passed, 5 total\n",
        "Tests:       2 failed, 15 passed, 17 total\n",
    );

    assert_eq!(
        test_tools::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn mocha_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("mocha_failing.txt");
    let expected = concat!(
        "1) divides by zero\n",
        "\n",
        "\n",
        "1 passing (4ms)\n",
        "1 failing\n",
        "1) math\n",
        "       divides by zero:\n",
        "\n",
        "      AssertionError [ERR_ASSERTION]: Expected values to be strictly equal:\n",
        "\n",
        "Infinity !== 0\n",
        "\n",
        "      + expected - actual\n",
        "\n",
        "      -Infinity\n",
        "      +0\n",
        "\n",
        "      at Context.<anonymous> (test/mocha.test.cjs:4:46)\n",
        "      at process.processImmediate (node:internal/timers:485:21)\n",
        "\n",
        "\n",
    );

    assert_eq!(
        test_tools::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn node_test_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("node_test_failing.txt");
    let expected = concat!(
        "not ok 2 - divides by zero\n",
        "  ---\n",
        "  duration_ms: 0.796\n",
        "  type: 'test'\n",
        "  location: '/tmp/smll-node-test/node.test.cjs:8:1'\n",
        "  failureType: 'testCodeFailure'\n",
        "  error: |-\n",
        "    Expected values to be strictly equal:\n",
        "\n",
        "    Infinity !== 0\n",
        "\n",
        "  code: 'ERR_ASSERTION'\n",
        "  name: 'AssertionError'\n",
        "  expected: 0\n",
        "  actual: Infinity\n",
        "  operator: 'strictEqual'\n",
        "  stack: |-\n",
        "    TestContext.<anonymous> (/tmp/smll-node-test/node.test.cjs:9:10)\n",
        "    Test.runInAsyncScope (node:async_hooks:214:14)\n",
        "    Test.run (node:internal/test_runner/test:1047:25)\n",
        "    Test.processPendingSubtests (node:internal/test_runner/test:744:18)\n",
        "    Test.postRun (node:internal/test_runner/test:1173:19)\n",
        "    Test.run (node:internal/test_runner/test:1101:12)\n",
        "    async startSubtestAfterBootstrap (node:internal/test_runner/harness:296:3)\n",
        "  ...\n",
        "1..2\n",
        "# tests 2\n",
        "# suites 0\n",
        "# pass 1\n",
        "# fail 1\n",
        "# cancelled 0\n",
        "# skipped 0\n",
        "# todo 0\n",
        "# duration_ms 47.74925\n",
    );

    assert_eq!(
        test_tools::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn tsc_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("tsc_errors.txt");
    let expected = concat!(
        "src/api/client.ts:42:5 TS2322: Type 'string' is not assignable to type 'number'.\n",
        "src/api/client.ts:58:12 TS2339: Property 'data' does not exist on type 'Response'.\n",
        "src/components/Button.tsx:15:7 TS2345: Argument of type 'undefined' is not assignable to parameter of type 'string'.\n",
        "src/utils/format.ts:8:3 TS7006: Parameter 'x' implicitly has an 'any' type.\n",
        "src/utils/format.ts:14:10 TS2304: Cannot find name 'Decimal'.\n",
        "Found 5 errors in 3 files.\n",
    );

    assert_eq!(
        test_tools::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn go_test_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("go_test_v.txt");
    let expected = concat!(
        "math_test.go:42: divide(10, 0) panic expected but got 0\n",
        "--- FAIL: TestDivide (0.00s)\n",
        "math_test.go:58: sqrt(-1) = 0, want NaN\n",
        "--- FAIL: TestSqrt (0.00s)\n",
        "FAIL\tgithub.com/example/math\t0.012s\n",
    );

    assert_eq!(
        test_tools::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn pytest_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("pytest_failing.txt");
    let expected = concat!(
        "test session starts\n",
        "platform darwin -- Python 3.12.0, pytest-8.0.0, pluggy-1.4.0\n",
        "rootdir: /Users/x/proj\n",
        "configfile: pyproject.toml\n",
        "plugins: anyio-4.2.0, cov-4.1.0\n",
        "collected 8 items\n",
        "FAILURES\n",
        "_______________________________ test_login_flow ________________________________\n",
        ">       assert resp.status_code == 200\n",
        "E       assert 401 == 200\n",
        "E        +  where 401 = <Response [401]>.status_code\n",
        ">       assert limiter.is_throttled()\n",
        "E       assert False\n",
        "E        +  where False = <bound method Limiter.is_throttled of <RateLimiter>>()\n",
        "short test summary info\n",
        "FAILED tests/test_api.py::test_login_flow - assert 401 == 200\n",
        "FAILED tests/test_api.py::test_rate_limit - assert False\n",
        "2 failed, 6 passed in 0.43s\n",
    );

    assert_eq!(
        test_tools::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn cargo_test_stream_dispatch_compacts_nonzero_actionable_output() {
    let input = fixture("cargo_test_failing.txt");
    let expected = test_tools::apply_matched(&input).unwrap();

    assert_eq!(
        test_tools::dispatch_streams_argv(&[b"cargo", b"test"], &input, b"", 101, false,).unwrap(),
        tapas::filters::StreamFilterOutput::new(
            expected.bytes,
            Vec::new(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn stream_dispatch_routes_direct_and_script_test_runners() {
    let cases: &[(&[&[u8]], &str)] = &[
        (&[b"jest"], "jest_failing.txt"),
        (&[b"vitest"], "jest_failing.txt"),
        (&[b"mocha"], "mocha_failing.txt"),
        (&[b"node", b"--test"], "node_test_failing.txt"),
        (&[b"tsc", b"--noEmit"], "tsc_errors.txt"),
        (&[b"go", b"test", b"-v"], "go_test_v.txt"),
        (&[b"pytest", b"-v"], "pytest_failing.txt"),
        (&[b"npm", b"test"], "jest_failing.txt"),
        (&[b"pnpm", b"test"], "mocha_failing.txt"),
        (&[b"yarn", b"test"], "jest_failing.txt"),
        (&[b"bun", b"test"], "node_test_failing.txt"),
    ];

    for &(argv, fixture_name) in cases {
        let input = fixture(fixture_name);
        let expected = test_tools::apply_matched(&input).unwrap();
        let output = test_tools::dispatch_streams_argv(argv, b"", &input, 1, false).unwrap();
        assert!(output.stdout.is_empty(), "argv {argv:?}");
        assert_eq!(output.stderr, expected.bytes, "argv {argv:?}");
        assert_eq!(
            output.evidence,
            EvidenceClass::FactComplete,
            "argv {argv:?}"
        );
    }
}

#[test]
fn stderr_only_test_output_stays_on_stderr() {
    let input = fixture("pytest_failing.txt");
    let expected = test_tools::apply_matched(&input).unwrap();

    assert_eq!(
        test_tools::dispatch_streams_argv(&[b"pytest", b"-v"], b"", &input, 1, false).unwrap(),
        tapas::filters::StreamFilterOutput::new(
            Vec::new(),
            expected.bytes,
            EvidenceClass::FactComplete,
        ),
    );
}

#[test]
fn test_truncation_notice_names_tapas_raw_mode() {
    let mut input = Vec::new();
    for index in 0..51 {
        input.extend_from_slice(format!("error: failure {index}\n").as_bytes());
        input.extend_from_slice(b"context one\ncontext two\ncontext three\n");
    }
    input.extend_from_slice(
        b"test result: FAILED. 0 passed; 51 failed; 0 ignored; finished in 0.1s\n",
    );

    let output =
        test_tools::dispatch_streams_argv(&[b"cargo", b"test"], &input, b"", 101, false).unwrap();

    assert!(
        output
            .stdout
            .windows(b"(tapas: omitted ".len())
            .any(|window| window == b"(tapas: omitted ")
    );
    assert!(
        output
            .stdout
            .windows(b"rerun with tapas --raw".len())
            .any(|window| window == b"rerun with tapas --raw")
    );
}

#[test]
fn stream_dispatch_preserves_lossless_queries_and_unknown_shapes_byte_exact() {
    let jest = fixture("jest_failing.txt");
    let stderr = b"\xffdiagnostic\n";
    let cases: &[(&[&[u8]], bool)] = &[
        (&[b"jest", b"--help"], false),
        (&[b"pytest", b"--collect-only"], false),
        (&[b"tsc", b"--showConfig"], false),
        (&[b"jest"], true),
    ];
    for &(argv, lossless) in cases {
        assert_eq!(
            test_tools::dispatch_streams_argv(argv, &jest, stderr, 1, lossless).unwrap(),
            tapas::filters::StreamFilterOutput::new(
                jest.clone(),
                stderr.to_vec(),
                EvidenceClass::ByteExact,
            ),
            "argv {argv:?} lossless {lossless}",
        );
    }

    let unknown_stdout = b"not a recognized test report \xff\n";
    let unknown_stderr = b"raw \xfe\n";
    assert_eq!(
        test_tools::dispatch_streams_argv(
            &[b"cargo", b"test"],
            unknown_stdout,
            unknown_stderr,
            1,
            false,
        )
        .unwrap(),
        tapas::filters::StreamFilterOutput::new(
            unknown_stdout.to_vec(),
            unknown_stderr.to_vec(),
            EvidenceClass::ByteExact,
        )
    );
    assert_eq!(
        test_tools::dispatch_streams_argv(&[], b"", b"", 0, false),
        Err(tapas::filters::FilterError::InvalidInput)
    );
}

#[test]
fn parsers_preserve_arbitrary_diagnostic_bytes_and_fail_open_on_unknown_input() {
    let tsc = b"src/a.ts:1:1 - error TS2322: bad \xff byte\nFound 1 error.\n";
    assert_eq!(
        test_tools::apply_matched(tsc).unwrap(),
        FilterOutput::new(
            b"src/a.ts:1:1 TS2322: bad \xff byte\nFound 1 error.\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    let malformed_tsc = b"error TSbroken \xfe\n";
    assert_eq!(
        test_tools::apply_matched(malformed_tsc).unwrap(),
        FilterOutput::new(malformed_tsc.to_vec(), EvidenceClass::FactComplete)
    );
    assert_eq!(
        test_tools::apply_matched(b"unrelated \xff bytes\n"),
        Err(tapas::filters::FilterError::InvalidInput)
    );
}

#[test]
fn pipe_dispatch_uses_the_pinned_first_match_order() {
    let overlapping = b"running 1 test\ntest x ... FAILED\ntest result: FAILED. 0 passed; 1 failed; finished in 0.1s\nTest Suites: 1 failed, 1 total\n";
    let output = test_tools::apply_matched(overlapping).unwrap();

    assert_eq!(output.evidence, EvidenceClass::FactComplete);
    assert!(output.bytes.ends_with(b"res 0p 1f 0.1s\n"));
    assert!(
        !output
            .bytes
            .windows(12)
            .any(|window| window == b"Test Suites:")
    );
}

#[test]
fn ctest_and_playwright_routes_require_known_finite_human_modes() {
    let ctest = fixture("ctest_success.txt");
    let ctest_output = test_tools::dispatch_streams_argv(
        &[b"ctest", b"--output-on-failure"],
        &ctest,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(ctest_output.evidence, EvidenceClass::PotentiallyLossy);
    assert_eq!(
        ctest_output.stdout,
        b"100% tests passed, 0 tests failed out of 2\nTotal Test time (real) =   0.03 sec\n"
    );

    let playwright = fixture("playwright_list_success.txt");
    for reporter in [b"list".as_slice(), b"line", b"dot"] {
        let argv = [b"playwright".as_slice(), b"test", b"--reporter", reporter];
        let output = test_tools::dispatch_streams_argv(&argv, &playwright, b"", 0, false).unwrap();
        assert_eq!(
            output.evidence,
            EvidenceClass::PotentiallyLossy,
            "{reporter:?}"
        );
        assert_eq!(
            output.stdout,
            b"Running 2 tests using 2 workers\n2 passed (1.2s)\n"
        );
    }

    for argv in [
        &[b"ctest".as_slice(), b"-N"][..],
        &[b"ctest".as_slice(), b"--output-junit", b"results.xml"][..],
        &[b"playwright".as_slice(), b"test"][..],
        &[b"playwright".as_slice(), b"test", b"--reporter=json"][..],
        &[b"playwright".as_slice(), b"test", b"--list"][..],
    ] {
        let input: &[u8] = if argv[0] == b"ctest" {
            &ctest
        } else {
            &playwright
        };
        let output = test_tools::dispatch_streams_argv(argv, input, b"raw\n", 0, false).unwrap();
        assert_eq!(output.evidence, EvidenceClass::ByteExact, "{argv:?}");
    }

    let terminator = test_tools::dispatch_streams_argv(
        &[
            b"playwright",
            b"test",
            b"--reporter=line",
            b"--",
            b"--reporter=json",
        ],
        &playwright,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(terminator.evidence, EvidenceClass::PotentiallyLossy);

    let failed = b"1) checkout > rejects invalid card\nerror: expected decline\ntrace detail\n";
    let output = test_tools::dispatch_streams_argv(
        &[b"playwright", b"test", b"--reporter=line"],
        b"",
        failed,
        1,
        false,
    )
    .unwrap();
    assert_eq!(output.evidence, EvidenceClass::ByteExact);
    assert_eq!(output.stderr, failed);
}
