use tapas::filters::{EvidenceClass, StreamFilterOutput, package};
mod common;
use common::fixture;

#[test]
fn npm_ls_fixture_matches_the_pinned_package_tree_oracle() {
    let input = fixture("npm_ls.txt");
    let expected = concat!(
        "demo-app@1.0.0 /private/repo\n",
        "deps +2: chalk@4.1.2, debug@4.3.4\n",
    );

    assert_eq!(
        package::dispatch_streams_argv(&[b"npm", b"ls"], &input, b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn package_tree_ecosystems_match_the_pinned_fixture_oracles() {
    let cases: &[(&[&[u8]], &str, &str)] = &[
        (
            &[b"npm", b"ls", b"--all"],
            "npm_ls_all.txt",
            concat!(
                "demo-app@1.0.0 /private/repo\n",
                "deps +2: chalk@4.1.2, debug@4.3.4\n",
                "nested rows x6\n",
            ),
        ),
        (
            &[b"pnpm", b"list"],
            "pnpm_list.txt",
            concat!(
                "demo-app@1.0.0 /private/repo (PRIVATE)\n",
                "deps +2: chalk@4.1.2, debug@4.3.4\n",
            ),
        ),
        (
            &[b"pnpm", b"list", b"--depth", b"5"],
            "pnpm_list_deep.txt",
            concat!(
                "demo-app@1.0.0 /private/repo (PRIVATE)\n",
                "deps +2: chalk@4.1.2, debug@4.3.4\n",
                "nested rows x6\n",
            ),
        ),
        (
            &[b"yarn", b"list"],
            "yarn_list.txt",
            concat!(
                "yarn list v1.22.22\n",
                "deps +8: ansi-styles@4.3.0, chalk@4.1.2, color-convert@2.0.1, color-name@1.1.4, debug@4.3.4, has-flag@4.0.0, ms@2.1.2, supports-color@7.2.0\n",
                "nested rows x6\n",
            ),
        ),
        (
            &[b"bun", b"pm", b"ls"],
            "bun_pm_ls.txt",
            concat!(
                "example-app@1.0.0 /Users/niels/projects/example-app node_modules (42)\n",
                "deps +14: @types/bun@1.1.6, @types/node@20.12.7, @types/react@18.3.3, @types/react-dom@18.3.0, discord.js@14.15.3, react@18.3.1, react-dom@18.3.1, typescript@5.4.5, zod@3.23.8, prisma@5.15.1, @prisma/client@5.15.1, dotenv@16.4.5, ...\n",
                "nested rows x19\n",
            ),
        ),
    ];

    for (argv, fixture_name, expected) in cases {
        let input = fixture(fixture_name);
        assert_eq!(
            package::dispatch_streams_argv(argv, &input, b"", 0, false).unwrap(),
            StreamFilterOutput::new(
                expected.as_bytes().to_vec(),
                Vec::new(),
                EvidenceClass::PotentiallyLossy,
            ),
            "fixture {fixture_name}",
        );
    }
}

#[test]
fn npm_install_fixture_matches_the_pinned_summary_oracle() {
    let input = fixture("npm_install.txt");
    let expected = concat!(
        "deprecated x5: lodash.isequal, rimraf, inflight, glob, querystring\n",
        "added 847 packages, and audited 848 packages in 12s\n",
        "found 2 vulnerabilities (1 moderate, 1 high)\n",
    );

    assert_eq!(
        package::dispatch_streams_argv(&[b"npm", b"install"], &input, b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn pnpm_bun_yarn_and_composer_install_fixtures_match_the_pinned_oracles() {
    let cases: &[(&[&[u8]], &str, &str)] = &[
        (
            &[b"pnpm", b"install"],
            "pnpm_install.txt",
            concat!(
                "deprecated x2: lodash.isequal, rimraf\n",
                "deps +2: react 18.2.0, react-dom 18.2.0\n",
                "dev +1: vite 5.0.0\n",
            ),
        ),
        (
            &[b"pnpm", b"i"],
            "pnpm9_install.txt",
            concat!(
                "The following dependencies have build scripts that were ignored: esbuild\n",
                "deps +2: react 18.2.0, react-dom 18.2.0\n",
                "dev +1: vite 5.0.0\n",
            ),
        ),
        (
            &[b"bun", b"add", b"react"],
            "bun_install.txt",
            concat!(
                "warn: deprecated lodash.isequal@4.5.0: This package is deprecated.\n",
                "warn: deprecated rimraf@3.0.2: Rimraf versions prior to v4 are no longer supported.\n",
                "deps +3: react@18.2.0, react-dom@18.2.0, vite@5.0.0\n",
                "3 packages installed [1.23s]\n",
            ),
        ),
        (
            &[b"yarn", b"add", b"react"],
            "yarn_install.txt",
            concat!(
                "warning \" > eslint-config-react@1.0.0\" has unmet peer dependency \"eslint@^8.0.0\".\n",
                "warning Workspaces can only be enabled in private projects.\n",
                "deps +3: react@18.2.0, react-dom@18.2.0, vite@5.0.0\n",
                "success Saved 3 new dependencies.\n",
                "Done in 5.32s.\n",
            ),
        ),
        (
            &[b"composer", b"require", b"guzzlehttp/guzzle"],
            "composer_require.txt",
            concat!(
                "Lock file operations: 4 installs, 0 updates, 0 removals\n",
                "Package operations: 4 installs, 0 updates, 0 removals\n",
                "No security vulnerability advisories found.\n",
                "Using version ^7.8 for guzzlehttp/guzzle\n",
            ),
        ),
    ];

    for (argv, fixture_name, expected) in cases {
        let input = fixture(fixture_name);
        assert_eq!(
            package::dispatch_streams_argv(argv, &input, b"", 0, false).unwrap(),
            StreamFilterOutput::new(
                expected.as_bytes().to_vec(),
                Vec::new(),
                EvidenceClass::PotentiallyLossy,
            ),
            "fixture {fixture_name}",
        );
    }
}

#[test]
fn pip_list_and_outdated_tables_preserve_every_row_fact() {
    let list = concat!(
        "Package    Version\n",
        "---------- -------\n",
        "requests   2.31.0\n",
        "urllib3    2.0.7\n",
    );
    assert_eq!(
        package::dispatch_streams_argv(&[b"pip", b"list"], list.as_bytes(), b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            b"requests 2.31.0\nurllib3 2.0.7\n".to_vec(),
            Vec::new(),
            EvidenceClass::FactComplete,
        ),
    );

    let outdated = concat!(
        "Package Version Latest Type\n",
        "------- ------- ------ -----\n",
        "pip     23.0    24.0   wheel\n",
    );
    assert_eq!(
        package::dispatch_streams_argv(
            &[b"pip3", b"list", b"--outdated"],
            outdated.as_bytes(),
            b"warning from index\n",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"pip 23.0 24.0 wheel\n".to_vec(),
            b"warning from index\n".to_vec(),
            EvidenceClass::FactComplete,
        ),
    );
}

#[test]
fn pip_install_summarizes_progress_and_keeps_the_result() {
    let input = concat!(
        "Collecting requests==2.31.0\n",
        "  Downloading requests-2.31.0-py3-none-any.whl (62 kB)\n",
        "     ---- 62.0/62.0 kB 3.2 MB/s eta 0:00:00\n",
        "Requirement already satisfied: urllib3<3,>=1.21.1 in /usr/lib/python3/dist-packages (from requests) (2.0.7)\n",
        "Installing collected packages: requests, urllib3\n",
        "Successfully installed requests-2.31.0 urllib3-2.0.7\n",
    );
    let expected = concat!(
        "Collecting 1: requests==2.31.0\n",
        "Downloaded 1 files\n",
        "Satisfied 1: urllib3<3,>=1.21.1\n",
        "Installing 2: requests, urllib3\n",
        "Successfully installed requests-2.31.0 urllib3-2.0.7\n",
    );
    assert_eq!(
        package::dispatch_streams_argv(
            &[b"pip", b"install", b"requests"],
            input.as_bytes(),
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn exact_query_machine_and_lossless_requests_preserve_both_streams() {
    let tree = fixture("npm_ls.txt");
    let stderr = b"diagnostic\n";
    let cases: &[&[&[u8]]] = &[
        &[b"npm", b"ls", b"--json"],
        &[b"pnpm", b"list", b"--format", b"json"],
        &[b"yarn", b"list", b"--reporter", b"ndjson"],
        &[b"pip", b"list", b"--format", b"json"],
        &[b"npm", b"ls", b"--help"],
        &[b"npm", b"help"],
        &[b"pip3", b"list", b"-h"],
    ];
    for argv in cases {
        assert_eq!(
            package::dispatch_streams_argv(argv, &tree, stderr, 0, false).unwrap(),
            StreamFilterOutput::new(tree.clone(), stderr.to_vec(), EvidenceClass::ByteExact,),
            "argv {argv:?}",
        );
    }

    let invalid = b"Collecting bad-\xff\n";
    assert_eq!(
        package::dispatch_streams_argv(&[b"pip", b"install", b"bad"], invalid, stderr, 0, true,)
            .unwrap(),
        StreamFilterOutput::new(invalid.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact,),
    );
}

#[test]
fn option_terminator_keeps_later_json_text_from_forcing_exact_mode() {
    let tree = fixture("npm_ls.txt");
    let output =
        package::dispatch_streams_argv(&[b"npm", b"ls", b"--", b"--json"], &tree, b"", 0, false)
            .unwrap();
    assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy);
    assert_eq!(
        output.stdout,
        b"demo-app@1.0.0 /private/repo\ndeps +2: chalk@4.1.2, debug@4.3.4\n"
    );
}

#[test]
fn nonzero_unrecognized_stderr_stays_raw_but_recognized_errors_are_owned() {
    let stdout = b"added 1 package in 1s\n";
    let unrecognized = b"connection closed unexpectedly\n";
    assert_eq!(
        package::dispatch_streams_argv(&[b"npm", b"install"], stdout, unrecognized, 1, false,)
            .unwrap(),
        StreamFilterOutput::new(
            stdout.to_vec(),
            unrecognized.to_vec(),
            EvidenceClass::ByteExact,
        ),
    );

    let recognized = b"npm ERR! code E404\nnpm ERR! 404 Not Found\n";
    assert_eq!(
        package::dispatch_streams_argv(&[b"npm", b"install"], b"", recognized, 1, false).unwrap(),
        StreamFilterOutput::new(Vec::new(), recognized.to_vec(), EvidenceClass::FactComplete,),
    );
}

#[test]
fn every_declared_package_action_routes_to_its_family_filter() {
    for action in [b"install".as_slice(), b"i", b"ci", b"add", b"remove", b"rm"] {
        for manager in [b"npm".as_slice(), b"pnpm", b"yarn", b"bun"] {
            let output = package::dispatch_streams_argv(
                &[manager, action],
                b"added 1 packages in 1s\n",
                b"",
                0,
                false,
            )
            .unwrap();
            assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy);
        }
    }

    for action in [
        b"install".as_slice(),
        b"require",
        b"update",
        b"upgrade",
        b"remove",
        b"create-project",
    ] {
        let output = package::dispatch_streams_argv(
            &[b"composer", action],
            b"Nothing to install, update or remove\n",
            b"",
            0,
            false,
        )
        .unwrap();
        assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy);
    }

    for action in [b"install".as_slice(), b"download", b"wheel"] {
        let output = package::dispatch_streams_argv(
            &[b"pip", action],
            b"Collecting example==1.0\n",
            b"",
            0,
            false,
        )
        .unwrap();
        assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy);
    }
}

#[test]
fn package_truncation_notice_names_tapas_raw_mode() {
    let mut input = Vec::new();
    for index in 0..61 {
        input.extend_from_slice(
            format!("Your requirements could not be resolved: conflict {index}\n").as_bytes(),
        );
    }

    let output =
        package::dispatch_streams_argv(&[b"composer", b"install"], &input, b"", 1, false).unwrap();

    assert!(
        output
            .stdout
            .windows(b"(tapas: omitted 1 relevant lines; rerun with tapas --raw)".len())
            .any(|window| window == b"(tapas: omitted 1 relevant lines; rerun with tapas --raw)")
    );
}

#[test]
fn uv_package_routes_compact_only_recognized_human_output() {
    let input = b"Resolved 5 packages in 20ms\nPrepared 2 packages in 10ms\nInstalled 2 packages in 4ms\n + flask==3.0.0\n + click==8.1.7\n";
    for argv in [
        &[b"uv".as_slice(), b"sync"][..],
        &[b"uv".as_slice(), b"add", b"flask"][..],
        &[b"uv".as_slice(), b"remove", b"flask"][..],
        &[b"uv".as_slice(), b"lock"][..],
        &[b"uv".as_slice(), b"tree"][..],
        &[b"uv".as_slice(), b"pip", b"install", b"flask"][..],
        &[b"uv".as_slice(), b"pip", b"sync", b"requirements.txt"][..],
        &[b"uv".as_slice(), b"pip", b"compile", b"requirements.in"][..],
    ] {
        let output = package::dispatch_streams_argv(argv, input, b"", 0, false).unwrap();
        assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy, "{argv:?}");
        assert!(output.stdout.starts_with(b"Resolved 5 packages in 20ms\n"));
    }

    for argv in [
        &[b"uv".as_slice(), b"sync", b"--json"][..],
        &[b"uv".as_slice(), b"sync", b"--", b"--json"][..],
        &[b"uvx".as_slice(), b"ruff"][..],
    ] {
        let output = package::dispatch_streams_argv(argv, input, b"", 0, false).unwrap();
        if argv.get(2) == Some(&b"--".as_slice()) {
            assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy);
        } else {
            assert_eq!(output.evidence, EvidenceClass::ByteExact, "{argv:?}");
        }
    }
}

#[test]
fn pip_exact_subcommands_and_option_terminators_follow_policy_boundaries() {
    let input = b"Collecting demo==1.0\nSuccessfully installed demo-1.0\n";
    for argv in [
        &[b"pip".as_slice(), b"freeze"][..],
        &[b"pip3".as_slice(), b"show", b"demo"][..],
        &[b"pip".as_slice(), b"check"][..],
        &[b"pip".as_slice(), b"inspect"][..],
        &[b"pip".as_slice(), b"list", b"--format=json"][..],
    ] {
        let output =
            package::dispatch_streams_argv(argv, input, b"diagnostic\n", 0, false).unwrap();
        assert_eq!(output.evidence, EvidenceClass::ByteExact, "{argv:?}");
    }

    let after_terminator = package::dispatch_streams_argv(
        &[b"pip", b"install", b"demo", b"--", b"--format=json"],
        input,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(after_terminator.evidence, EvidenceClass::PotentiallyLossy);

    let malformed = package::dispatch_streams_argv(
        &[b"uv", b"sync"],
        b"Resolved 1 package\n\xff",
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(malformed.evidence, EvidenceClass::ByteExact);

    let failed = b"error: failed to resolve dependencies\nCaused by: registry unavailable\n";
    let output = package::dispatch_streams_argv(&[b"uv", b"sync"], b"", failed, 1, false).unwrap();
    assert_eq!(output.evidence, EvidenceClass::ByteExact);
    assert_eq!(output.stderr, failed);
}
