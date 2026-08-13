use tapas::filters::{EvidenceClass, FilterOutput, listing};
mod common;
use common::fixture;

type RouteCase<'a> = (&'a [&'a [u8]], &'a [u8], i32, bool);

#[test]
fn tree_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("tree_src.txt");
    let expected = concat!(
        "src/\n",
        "  filters\n",
        "    detect.zig\n",
        "~git_add.zig\n",
        "~git_blame.zig\n",
        "~git_branch.zig\n",
        "~git_checkout.zig\n",
        "~git_commit.zig\n",
        "~git_diff.zig\n",
        "~git_fetch.zig\n",
        "~git_log.zig\n",
        "~git_merge.zig\n",
        "~git_pull.zig\n",
        "~git_push.zig\n",
        "~git_rebase.zig\n",
        "~git_show.zig\n",
        "~git_stash.zig\n",
        "~git_status.zig\n",
        "~rg.zig\n",
        "~sigil_rle.zig\n",
        "~validator.zig\n",
        "  main.zig\n",
        "~pipeline.zig\n",
        "~util.zig\n",
        "\n",
        "2 directories, 22 files\n",
    );

    assert!(listing::matches(&input));
    assert_eq!(
        listing::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.as_bytes().to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn grep_compacts_only_large_path_prefixed_multi_file_human_matches() {
    let input = fixture("grep_multifile.txt");
    let output = listing::dispatch_streams_argv(
        &[b"grep", b"-nH", b"needle", b"src/a.rs", b"src/b.rs"],
        &input,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy);
    for needle in [
        b"src/a.rs:0: needle alpha\n".as_slice(),
        b"src/a.rs: ... 12 more matches\n",
        b"src/b.rs: ... 12 more matches\n",
    ] {
        assert!(
            output
                .stdout
                .windows(needle.len())
                .any(|part| part == needle)
        );
    }

    for argv in [
        &[
            b"grep".as_slice(),
            b"-c",
            b"needle",
            b"src/a.rs",
            b"src/b.rs",
        ][..],
        &[
            b"grep".as_slice(),
            b"-o",
            b"needle",
            b"src/a.rs",
            b"src/b.rs",
        ][..],
        &[
            b"grep".as_slice(),
            b"-A",
            b"2",
            b"needle",
            b"src/a.rs",
            b"src/b.rs",
        ][..],
        &[b"grep".as_slice(), b"needle", b"src/a.rs"][..],
    ] {
        let exact = listing::dispatch_streams_argv(argv, &input, b"", 0, false).unwrap();
        assert_eq!(exact.evidence, EvidenceClass::ByteExact, "{argv:?}");
    }

    let no_match = listing::dispatch_streams_argv(
        &[b"grep", b"needle", b"src/a.rs", b"src/b.rs"],
        b"",
        b"",
        1,
        false,
    )
    .unwrap();
    assert_eq!(no_match.evidence, EvidenceClass::ByteExact);

    let terminated = listing::dispatch_streams_argv(
        &[
            b"grep",
            b"needle",
            b"src/a.rs",
            b"src/b.rs",
            b"--",
            b"--count",
        ],
        &input,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(terminated.evidence, EvidenceClass::PotentiallyLossy);

    let colon_path_input = (0..10)
        .flat_map(|line| {
            [
                format!("src/a:part.rs:{line}: needle alpha\n"),
                format!("src/b.rs:{line}: needle beta\n"),
            ]
        })
        .collect::<String>()
        .into_bytes();
    let colon_path = listing::dispatch_streams_argv(
        &[b"grep", b"-nH", b"needle", b"src/a:part.rs", b"src/b.rs"],
        &colon_path_input,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(colon_path.evidence, EvidenceClass::ByteExact);
    assert_eq!(colon_path.stdout, colon_path_input);

    let colon_pattern = listing::dispatch_streams_argv(
        &[b"grep", b"-nH", b"needle: alpha", b"src/a.rs", b"src/b.rs"],
        &input,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(colon_pattern.evidence, EvidenceClass::PotentiallyLossy);
}

#[test]
fn ls_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("ls_la.txt");
    let expected = b"filters/\nmain.zig\npipeline.zig\nutil.zig\n";

    assert!(listing::matches(&input));
    assert_eq!(
        listing::apply_matched(&input).unwrap(),
        FilterOutput::new(expected.to_vec(), EvidenceClass::PotentiallyLossy)
    );
}

#[test]
fn find_ls_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("find_ls.txt");
    let expected = concat!(
        "./ (3 entries: README.md, src/, tests/)\n",
        "./src/filter.zig\n",
        "./src/main.zig\n",
    );

    assert!(listing::matches(&input));
    assert_eq!(
        listing::apply_matched(&input).unwrap(),
        FilterOutput::new(
            expected.as_bytes().to_vec(),
            EvidenceClass::PotentiallyLossy
        )
    );
}

#[test]
fn find_ls_pipe_filter_preserves_the_final_group_entry() {
    let input = concat!(
        "1 0 -rw-r--r-- 1 user staff 1 Apr 23 12:34 ./src/a.zig\n",
        "2 0 -rw-r--r-- 1 user staff 1 Apr 23 12:34 ./src/b.zig\n",
        "3 0 -rw-r--r-- 1 user staff 1 Apr 23 12:34 ./src/c.zig\n",
        "4 0 -rw-r--r-- 1 user staff 1 Apr 23 12:34 ./src/d.zig\n",
        "5 0 drwxr-xr-x 1 user staff 1 Apr 23 12:34 ./src/e.zig\n",
    );
    let expected =
        b"./src/ (5 entries: a.zig, b.zig, c.zig; 1 omitted; last: e.zig/; --raw for all)\n";

    assert_eq!(
        listing::apply_matched(input.as_bytes()).unwrap(),
        FilterOutput::new(expected.to_vec(), EvidenceClass::PotentiallyLossy)
    );
}

#[test]
fn du_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("du_sh.txt");
    let expected = concat!(
        "5G\t./node_modules\n",
        "1.2G\t./vendor\n",
        "230M\t./src\n",
        "82M\t./build\n",
        "17K\t./tests\n",
        "4.0K\t./docs\n",
    );

    assert!(listing::matches(&input));
    assert_eq!(
        listing::apply_matched(&input).unwrap(),
        FilterOutput::new(
            expected.as_bytes().to_vec(),
            EvidenceClass::PotentiallyLossy
        )
    );
}

#[test]
fn rg_pattern_dispatch_elides_only_repeated_paths_losslessly() {
    let stdout = concat!(
        "src/main.rs:7:needle one\n",
        "src/main.rs:12:needle two\n",
        "--\n",
        "src/main.rs:21:needle three\n",
        "src/lib.rs:4:needle four\n",
        "src/lib.rs:9:needle five\n",
    );
    let expected = concat!(
        "src/main.rs:7:needle one\n",
        ":12:needle two\n",
        "--\n",
        "src/main.rs:21:needle three\n",
        "src/lib.rs:4:needle four\n",
        ":9:needle five\n",
    );

    assert_eq!(
        listing::dispatch_streams_argv(
            &[b"rg", b"needle"],
            stdout.as_bytes(),
            b"warning \xff\n",
            0,
            false,
        )
        .unwrap(),
        tapas::filters::StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            b"warning \xff\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn rg_machine_file_modes_are_byte_exact() {
    let input = fixture("rg_files.txt");
    for flag in [
        b"--files".as_slice(),
        b"-l".as_slice(),
        b"--files-with-matches".as_slice(),
    ] {
        assert_eq!(
            listing::dispatch_streams_argv(&[b"rg", flag], &input, b"", 0, false).unwrap(),
            tapas::filters::StreamFilterOutput::new(
                input.clone(),
                Vec::new(),
                EvidenceClass::ByteExact,
            ),
            "flag {flag:?}",
        );
    }
}

#[test]
fn find_plain_dispatch_preserves_group_boundaries() {
    let input = fixture("find_plain_many.txt");
    let expected = concat!(
        "README.md\n",
        "build.zig\n",
        "scripts/audit-fixtures.py\n",
        "src/core/ (12 entries: analyzer.zig, cache.zig, config.zig; 8 omitted; last: vm.zig; --raw for all)\n",
        "src/filters/ (12 entries: build_output.zig, cargo_test.zig, find_compact.zig; 8 omitted; last: validator.zig; --raw for all)\n",
        "tests/fixtures/ (12 entries: build_output.txt, cargo_test_failing.txt, find_plain_many.txt; 8 omitted; last: tree_large.txt; --raw for all)\n",
    );

    assert_eq!(
        listing::dispatch_streams_argv(&[b"find", b"."], &input, b"", 0, false).unwrap(),
        tapas::filters::StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )
    );

    let expected_files = expected.replace(" entries:", " files:");
    assert_eq!(
        listing::dispatch_streams_argv(&[b"find", b".", b"-type", b"f"], &input, b"", 0, false,)
            .unwrap()
            .stdout,
        expected_files.as_bytes(),
    );
}

#[test]
fn find_plain_four_entry_group_exposes_every_boundary_byte() {
    let input = b"dir/a\ndir/b\ndir/c\ndir/\xff\n";
    let expected = b"dir/ (4 entries: a, b, c; last: \xff)\n";

    assert_eq!(
        listing::dispatch_streams_argv(&[b"find", b"."], input, b"", 0, false)
            .unwrap()
            .stdout,
        expected,
    );
}

#[test]
fn tree_dispatch_matches_the_pinned_compact_oracle() {
    let input = fixture("tree_src.txt");
    let expected = concat!(
        "src/\n",
        "  filters/ (19 files: detect.zig, git_add.zig, git_blame.zig; 16 omitted; --raw for all)\n",
        "  main.zig\n",
        "  pipeline.zig\n",
        "  util.zig\n",
        "2 directories, 22 files\n",
    );

    assert_eq!(
        listing::dispatch_streams_argv(&[b"tree"], &input, b"", 0, false).unwrap(),
        tapas::filters::StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn ls_column_dispatch_matches_the_pinned_fixture_output() {
    let input = fixture("ls_columns.txt");
    let expected = concat!(
        "filter_catalog.zig\n",
        "filters\n",
        "history.zig\n",
        "main.zig\n",
        "pipe_filters.zig\n",
        "pipeline.zig\n",
        "setup.zig\n",
        "setup_hooks.zig\n",
        "setup_io.zig\n",
        "setup_json.zig\n",
        "signals.zig\n",
        "stats.zig\n",
        "tee.zig\n",
        "util.zig\n",
        "wrapper.zig\n",
        "wrapper_git.zig\n",
        "wrapper_io.zig\n",
        "wrapper_util.zig\n",
    );

    assert_eq!(
        listing::dispatch_streams_argv(&[b"ls", b"-C"], &input, b"", 0, false).unwrap(),
        tapas::filters::StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn ls_block_modes_match_the_pinned_fixture_outputs() {
    let multi = fixture("ls_multi_dir.txt");
    let expected_multi = concat!(
        "docs/ (7 entries: audits, brainstorms, drafts; 4 omitted; --raw for all)\n",
        "src/ (18 entries: filter_catalog.zig, filters, history.zig; 15 omitted; --raw for all)\n",
    );
    assert_eq!(
        listing::dispatch_streams_argv(&[b"ls", b"docs", b"src"], &multi, b"", 0, false)
            .unwrap()
            .stdout,
        expected_multi.as_bytes(),
    );

    let recursive = fixture("ls_recursive.txt");
    let expected_recursive = concat!(
        "filter_catalog.zig\nfilters\nhistory.zig\nmain.zig\npipe_filters.zig\n",
        "pipeline.zig\nsetup_hooks.zig\nsetup_io.zig\nsetup_json.zig\nsetup.zig\n",
        "signals.zig\nstats.zig\ntee.zig\nutil.zig\nwrapper_git.zig\nwrapper_io.zig\n",
        "wrapper_util.zig\nwrapper.zig\n",
        "src/filters/ (57 entries: ansi.zig, build_compact.zig, build_output.zig; 54 omitted; --raw for all)\n",
    );
    assert_eq!(
        listing::dispatch_streams_argv(&[b"ls", b"-R", b"src"], &recursive, b"", 0, false)
            .unwrap()
            .stdout,
        expected_recursive.as_bytes(),
    );
}

#[test]
fn du_dispatch_rounds_and_only_sorts_summarized_output() {
    let input = b"1.2G\t./vendor\n234M\t./build\n17K\t./src\n5G\t./huge\n";
    let expected_in_order = b"1.2G\t./vendor\n230M\t./build\n17K\t./src\n5G\t./huge\n";
    let expected_sorted = b"5G\t./huge\n1.2G\t./vendor\n230M\t./build\n17K\t./src\n";

    assert_eq!(
        listing::dispatch_streams_argv(&[b"du", b"-h"], input, b"", 0, false)
            .unwrap()
            .stdout,
        expected_in_order,
    );
    assert_eq!(
        listing::dispatch_streams_argv(&[b"du", b"-sh"], input, b"", 0, false)
            .unwrap()
            .stdout,
        expected_sorted,
    );
}

#[test]
fn wc_dispatch_collapses_padding_and_preserves_diagnostics() {
    assert_eq!(
        listing::dispatch_streams_argv(
            &[b"wc", b"-l", b"file.txt"],
            b"      12      34     567 file.txt\n",
            b"wc: bad \xff path\n",
            1,
            false,
        )
        .unwrap(),
        tapas::filters::StreamFilterOutput::new(
            b"12 34 567 file.txt\n".to_vec(),
            b"wc: bad \xff path\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn env_dispatch_masks_secrets_and_only_filters_listing_forms() {
    let stdout = b"HOME=/tmp/example\nAPI_TOKEN=secret-value\nEMPTY_TOKEN=\nNORMAL=bad-\xff-byte\n";
    let expected =
        b"HOME=/tmp/example\nAPI_TOKEN=se****ue\nEMPTY_TOKEN=****\nNORMAL=bad-\xff-byte\n";
    assert_eq!(
        listing::dispatch_streams_argv(&[b"env"], stdout, b"env: warning\n", 0, false).unwrap(),
        tapas::filters::StreamFilterOutput::new(
            expected.to_vec(),
            b"env: warning\n".to_vec(),
            EvidenceClass::PotentiallyLossy,
        )
    );

    assert_eq!(
        listing::dispatch_streams_argv(&[b"env", b"FOO=bar", b"command"], stdout, b"", 0, false,)
            .unwrap()
            .evidence,
        EvidenceClass::ByteExact,
    );
}

#[test]
fn lossless_exact_modes_unknown_shapes_and_parser_failures_are_byte_exact() {
    type BypassCase<'a> = (&'a [&'a [u8]], &'a [u8], bool);

    let rg = fixture("rg_files.txt");
    let tree = fixture("tree_src.txt");
    let find_ls = fixture("find_ls.txt");
    let ls = fixture("ls_la.txt");
    let du = fixture("du_sh.txt");
    let stderr = b"diagnostic \xff\n";
    let cases: &[BypassCase<'_>] = &[
        (&[b"rg", b"--json"], &rg, false),
        (&[b"find", b".", b"-ls"], &find_ls, false),
        (&[b"tree", b"-J"], &tree, false),
        (&[b"ls", b"-l"], &ls, false),
        (&[b"du", b"--help"], &du, false),
        (&[b"wc", b"--version"], b"      42 file\n", false),
        (&[b"env", b"--help"], b"API_TOKEN=secret\n", false),
        (&[b"tree"], &tree, true),
        (&[b"unknown"], b"raw \xfe\n", false),
        (
            &[b"ls"],
            b"total 8\ndrwxr-xr-x - user 22 Apr 14:30 src\n",
            false,
        ),
        (&[b"du"], b"17K\t./src\nmalformed\n", false),
        (&[b"find", b"."], b" good\n", false),
    ];

    for &(argv, stdout, lossless) in cases {
        assert_eq!(
            listing::dispatch_streams_argv(argv, stdout, stderr, 1, lossless).unwrap(),
            tapas::filters::StreamFilterOutput::new(
                stdout.to_vec(),
                stderr.to_vec(),
                EvidenceClass::ByteExact,
            ),
            "argv {argv:?}",
        );
    }

    assert_eq!(
        listing::dispatch_streams_argv(&[], b"", b"", 0, false),
        Err(tapas::filters::FilterError::InvalidInput),
    );
    assert_eq!(
        listing::apply_matched(b"unrelated \xff bytes\n"),
        Err(tapas::filters::FilterError::InvalidInput),
    );
}

#[test]
fn direct_diff_compacts_recognized_normal_and_unified_human_shapes() {
    let normal = b"1c1\n< old value\n---\n> new value\n";
    assert_eq!(
        listing::dispatch_streams_argv(
            &[b"/usr/bin/diff", b"old.txt", b"new.txt"],
            normal,
            b"locale notice\n",
            1,
            false,
        )
        .unwrap(),
        tapas::filters::StreamFilterOutput::new(
            b"@1c1\n-old value\n+new value\n".to_vec(),
            b"locale notice\n".to_vec(),
            EvidenceClass::FactComplete,
        ),
    );

    let unified = b"--- old.txt\t2026-08-13 10:00:00\n+++ new.txt\t2026-08-13 10:01:00\n@@ -1,2 +1,2 @@\n same\n-old\n+new\n";
    assert_eq!(
        listing::dispatch_streams_argv(
            &[b"diff", b"old.txt", b"new.txt"],
            unified,
            b"",
            1,
            false,
        )
        .unwrap(),
        tapas::filters::StreamFilterOutput::new(
            b"--- old.txt\t2026-08-13 10:00:00\n+++ new.txt\t2026-08-13 10:01:00\n@1,2|1,2\n same\n-old\n+new\n".to_vec(),
            Vec::new(),
            EvidenceClass::FactComplete,
        ),
    );
}

#[test]
fn direct_diff_owns_unknown_exact_non_utf8_and_failed_shapes_as_passthrough() {
    let stderr = b"diff diagnostic \xff\n";
    let cases: &[RouteCase<'_>] = &[
        (&[b"diff", b"a", b"b"], b"1c1\nmalformed\n", 1, false),
        (
            &[b"diff", b"a", b"b"],
            b"Binary files a and b differ\n",
            1,
            false,
        ),
        (&[b"diff", b"a", b"b"], b"1c1\n< \xff\n---\n> x\n", 1, false),
        (
            &[b"diff", b"--unified", b"a", b"b"],
            b"--- a\n+++ b\n",
            1,
            false,
        ),
        (&[b"diff", b"a", b"b"], b"diff: trouble\n", 2, false),
        (&[b"diff", b"a", b"b"], b"1a2\n> x\n", 1, true),
    ];
    for &(argv, stdout, exit_code, lossless) in cases {
        assert_eq!(
            listing::dispatch_streams_argv(argv, stdout, stderr, exit_code, lossless).unwrap(),
            tapas::filters::StreamFilterOutput::passthrough(stdout, stderr),
            "argv {argv:?}",
        );
    }
}

#[test]
fn direct_head_and_tail_compact_default_text_and_preserve_stderr() {
    let input = b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let expected = b"one\ntwo\nthree\nfour\n... 3 lines omitted ...\neight\nnine\nten\n";
    for command in [b"head".as_slice(), b"tail"] {
        assert_eq!(
            listing::dispatch_streams_argv(
                &[command, b"notes.txt"],
                input,
                b"read warning\n",
                0,
                false,
            )
            .unwrap(),
            tapas::filters::StreamFilterOutput::new(
                expected.to_vec(),
                b"read warning\n".to_vec(),
                EvidenceClass::PotentiallyLossy,
            ),
            "command {command:?}",
        );
    }
}

#[test]
fn direct_head_and_tail_own_flags_unknown_binary_non_utf8_and_failures_as_passthrough() {
    let text = b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let stderr = b"diagnostic \xff\n";
    let cases: &[RouteCase<'_>] = &[
        (&[b"head", b"-n", b"5", b"file"], text, 0, false),
        (&[b"tail", b"-n5", b"file"], text, 0, false),
        (&[b"head"], text, 0, false),
        (&[b"tail", b"a", b"b"], text, 0, false),
        (&[b"head", b"file"], b"short text\n", 0, false),
        (&[b"tail", b"file"], b"one\n\xff\n", 0, false),
        (&[b"head", b"file"], b"one\0two\n", 0, false),
        (&[b"tail", b"file"], text, 1, false),
        (&[b"head", b"file"], text, 0, true),
    ];
    for &(argv, stdout, exit_code, lossless) in cases {
        assert_eq!(
            listing::dispatch_streams_argv(argv, stdout, stderr, exit_code, lossless).unwrap(),
            tapas::filters::StreamFilterOutput::passthrough(stdout, stderr),
            "argv {argv:?}",
        );
    }
}
