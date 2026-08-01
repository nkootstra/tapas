use std::io::Cursor;

use tapas::filters::{EvidenceClass, FilterError, FilterOutput};
use tapas::pipeline::{self, FilterSpec, MAX_PIPE_INPUT_BYTES};
use tapas::signals::Signals;

fn always(_: &[u8]) -> bool {
    true
}

fn first(_: &[u8]) -> Result<FilterOutput, FilterError> {
    Ok(FilterOutput::new(
        b"first\n".to_vec(),
        EvidenceClass::FactComplete,
    ))
}

fn second(_: &[u8]) -> Result<FilterOutput, FilterError> {
    Ok(FilterOutput::new(
        b"second\n".to_vec(),
        EvidenceClass::FactComplete,
    ))
}

fn fails_after_candidate(_: &[u8]) -> Result<FilterOutput, FilterError> {
    let _discarded_candidate = b"partial compact output".to_vec();
    Err(FilterError::InvalidInput)
}

#[test]
fn empty_and_arbitrary_bytes_pass_through() {
    assert_eq!(pipeline::filter_bytes(b""), b"");

    let bytes: Vec<u8> = (0..=u8::MAX).collect();
    assert_eq!(pipeline::filter_bytes(&bytes), bytes);

    let invalid = b"valid prefix\xff\xfe  repeated   spaces\n";
    assert_eq!(pipeline::filter_bytes(invalid), invalid);
}

#[test]
fn first_match_wins_and_filter_errors_are_transactional() {
    let first_match = [
        FilterSpec::new("first", tapas::filters::generic::always, always, first),
        FilterSpec::new("second", tapas::filters::generic::always, always, second),
    ];
    assert_eq!(
        pipeline::dispatch_with_filters(b"long raw input\n", &first_match),
        b"first\n"
    );

    let failing = [FilterSpec::new(
        "failing",
        tapas::filters::generic::always,
        always,
        fails_after_candidate,
    )];
    assert_eq!(
        pipeline::dispatch_with_filters(b"raw\n", &failing),
        b"raw\n"
    );
}

#[test]
fn inspection_limit_transitions_to_raw_without_duplication() {
    let below = vec![0; MAX_PIPE_INPUT_BYTES - 1];
    let exact = vec![b'x'; MAX_PIPE_INPUT_BYTES];
    let above = vec![b'x'; MAX_PIPE_INPUT_BYTES + 1];

    let mut below_out = Vec::new();
    pipeline::run(&mut Cursor::new(&below), &mut below_out).unwrap();
    assert_eq!(below_out, below);

    for input in [&exact, &above] {
        let mut output = Vec::new();
        pipeline::run(&mut Cursor::new(input), &mut output).unwrap();
        assert_eq!(&output, input);
    }
}

#[test]
fn json_passthrough_and_generic_text_compaction_match_the_oracle() {
    let json = b"{\n  \"message\": \"x y\",\n  \"count\": 2\n}\n";
    assert_eq!(pipeline::filter_bytes(json), json);

    let mut large_json = b"{\n  \"items\": [\n".to_vec();
    large_json.extend(std::iter::repeat_n(b' ', 5 * 1024));
    large_json.extend_from_slice(b"]\n}\n");
    assert_eq!(pipeline::filter_bytes(&large_json), large_json);

    let mut noisy = Vec::new();
    for _ in 0..300 {
        noisy.extend_from_slice(b"\x1b[32mbuild    complete\x1b[0m   \n\n");
    }
    let compact = pipeline::filter_bytes(&noisy);
    let rendered = String::from_utf8(compact).unwrap();
    assert!(rendered.contains("build complete"));
    assert!(rendered.contains("300"));
    assert!(!rendered.contains("\x1b["));

    let dispatch = pipeline::filter(&noisy);
    assert_eq!(dispatch.filter_name, "generic");
    assert_eq!(dispatch.evidence, EvidenceClass::FactComplete);

    let repeated_invalid = b"invalid \xff\xfe diagnostic\n".repeat(300);
    assert_eq!(
        pipeline::filter_bytes(&repeated_invalid),
        b"invalid \xff\xfe diagnostic \xc3\x97300\n"
    );
}

#[test]
fn deterministic_random_bytes_fail_open() {
    let mut state = 0x9e37_79b9_u32;
    let mut bytes = Vec::with_capacity(8 * 1024);
    for _ in 0..bytes.capacity() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        bytes.push((state >> 24) as u8);
    }

    let dispatch = pipeline::filter(&bytes);
    assert_eq!(dispatch.bytes, bytes);
    assert_eq!(dispatch.filter_name, "passthrough");
    assert_eq!(dispatch.evidence, EvidenceClass::ByteExact);
}

#[test]
fn signal_classifier_gates_are_supersets_for_known_needles() {
    let samples = [
        (
            b"test result: FAILED".as_slice(),
            Signals::cargo_test as fn(Signals) -> bool,
        ),
        (b"Test Suites: 1 failed".as_slice(), Signals::jest),
        (b"src/main.ts(1,2): error TS1234".as_slice(), Signals::tsc),
        (
            b"=== RUN TestThing\n--- FAIL: TestThing".as_slice(),
            Signals::go_test,
        ),
        (
            b"test session starts\n1 failed in 0.2s".as_slice(),
            Signals::pytest,
        ),
        (b"added 42 packages".as_slice(), Signals::npm_install),
    ];

    for (input, gate) in samples {
        assert!(
            gate(Signals::compute(input)),
            "missing signal for {input:?}"
        );
    }
}

#[test]
fn signal_classifier_accepts_the_frozen_positive_fixtures() {
    let fixtures = [
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/cargo_test_failing.txt")
                .as_slice(),
            Signals::cargo_test as fn(Signals) -> bool,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/jest_failing.txt")
                .as_slice(),
            Signals::jest,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/mocha_failing.txt")
                .as_slice(),
            Signals::js_test,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/node_test_failing.txt")
                .as_slice(),
            Signals::js_test,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/tsc_errors.txt").as_slice(),
            Signals::tsc,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/go_test_v.txt").as_slice(),
            Signals::go_test,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/pytest_failing.txt")
                .as_slice(),
            Signals::pytest,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/npm_install.txt").as_slice(),
            Signals::npm_install,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/pnpm_install.txt")
                .as_slice(),
            Signals::npm_install,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/bun_install.txt").as_slice(),
            Signals::npm_install,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/yarn_install.txt")
                .as_slice(),
            Signals::npm_install,
        ),
        (
            include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/composer_require.txt")
                .as_slice(),
            Signals::npm_install,
        ),
    ];

    for (input, gate) in fixtures {
        assert!(gate(Signals::compute(input)));
    }
}

#[test]
fn pipe_chain_matches_container_package_and_curl_oracles() {
    let kubectl = include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/kubectl_pods.txt");
    let kubectl_output = pipeline::filter_bytes(kubectl);
    assert!(kubectl_output.starts_with(b"k9r "));

    let docker = include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/docker_ps.txt");
    assert_eq!(
        pipeline::filter_bytes(docker),
        b"d4up helios-assistant helios-convex-dashboard helios-convex-backend helios-mysql\n"
    );

    let images = include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/docker_images.txt");
    assert_eq!(pipeline::filter_bytes(images), images);

    let npm = include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/npm_install.txt");
    assert_eq!(
        pipeline::filter_bytes(npm),
        concat!(
            "deprecated x5: lodash.isequal, rimraf, inflight, glob, querystring\n",
            "added 847 packages, and audited 848 packages in 12s\n",
            "found 2 vulnerabilities (1 moderate, 1 high)\n",
        )
        .as_bytes()
    );

    let curl =
        include_bytes!("compat/smll-v1.9.0/fixtures/tests/fixtures/curl_v_example.stderr.txt");
    assert_eq!(
        pipeline::filter_bytes(curl),
        concat!(
            "* Connected to example.com (93.184.216.34) port 443\n",
            "> GET / HTTP/2\n",
            "> Host: example.com\n",
            "> User-Agent: curl/8.0.1\n",
            "> accept: */*\n",
            ">\n",
            "< HTTP/2 200\n",
            "< content-type: text/html; charset=UTF-8\n",
            "< content-length: 182\n",
            "< date: Mon, 22 Apr 2024 12:00:00 GMT\n",
            "<\n",
        )
        .as_bytes()
    );
}
