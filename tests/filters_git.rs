use tapas::filters::{EvidenceClass, StreamFilterOutput, git};
mod common;
use common::fixture;

#[test]
fn diff_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_diff_simple.txt");

    assert!(git::matches(&input));
    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            b"d simple.txt\n@1|1,3\n line one\n+line two\n+line three\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn diff_pipe_matcher_finds_a_diff_after_a_preamble() {
    let input = b"preamble\ndiff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\n";

    assert!(git::matches(input));
    assert_eq!(
        git::apply_matched(input).unwrap(),
        tapas::filters::FilterOutput::new(
            b"preamble\nd a.txt\n@1|1\n-old\n+new\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn status_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_status_dirty.txt");

    assert!(git::matches(&input));
    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            b"# main =origin/main\nM src/main.zig\nM src/pipeline.zig\n? src/filters/git_status.zig\n? tests/fixtures/git_status_dirty.txt\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn log_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_log_linear.txt");

    assert!(git::matches(&input));
    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            b"f0ad49e fix: third line\nf666a84 feat: extend a.txt\n95cbeda feat: add a.txt with one line\n".to_vec(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn show_wins_before_log_and_matches_the_pinned_oracle() {
    let input = fixture("git_show_body.txt");

    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            b"f666a84 feat: extend a.txt\n  This body explains why we added a second line.\n  It spans multiple lines and contains punctuation.\n\nd a.txt\n@1|1,2\n line1\n+line2\n".to_vec(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn branch_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_branch_list.txt");

    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            b" feature-x\n feature-y\n* main\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn reflog_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_reflog.txt");
    let expected = b"1a1f2f7 @0 checkout: moving from main to main\n\
~ @1 pull --ff-only origin main: Fast-forward\n\
1edb490 @2 checkout: moving from harden-workflow-actions to main\n\
a8b4598 @3 commit: ci: pin actions to commit SHAs and harden tag-derived version interpolation\n\
1edb490 @4 checkout: moving from main to harden-workflow-actions\n\
~ @5 pull --ff-only origin main: Fast-forward\n\
78a9d9e @6 checkout: moving from feat/generic-output-optimization to main\n\
474d0d9 @7 commit: fix: preserve actionable gh output\n\
9ce95e8 @8 commit: docs: document generic table fallback\n\
5a7170a @9 commit: feat: optimize generic table output\n\
78a9d9e @10 checkout: moving from main to feat/generic-output-optimization\n\
~ @11 commit: chore: release v1.2.5\n";

    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(expected.to_vec(), EvidenceClass::FactComplete)
    );
}

#[test]
fn commit_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_commit_simple.txt");

    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            b"c 6b68b6d main feat: add a.txt\n+1/-0 files=1\n+ a.txt\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn merge_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_merge_ff.txt");

    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            b"@ ff 81a7b77..af90dc8\nfx.txt |1\n+1/-0 files=1\n+ fx.txt\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn blame_pipe_filter_matches_the_pinned_oracle() {
    let input = fixture("git_blame_simple.txt");
    let expected = concat!(
        "b 17d6c4f 2026-01-01 Alice Anderson\n",
        " fn init() {\n",
        "     // initialise the module\n",
        "     setup_defaults();\n",
        "b 6ac6cdd\n",
        "     configure_logging();\n",
        "     configure_metrics();\n",
        "     bind_signals();\n",
        "b 9e6e8fe\n",
        "     start_event_loop();\n",
        "     drain_queue();\n",
        "     flush_buffers();\n",
        "b f34c3be\n",
        "     persist_state();\n",
        "     checkpoint();\n",
        "     notify_ready();\n",
        "b 526c207\n",
        "     wait_for_shutdown();\n",
        "     teardown();\n",
        " }\n",
    );

    assert_eq!(
        git::apply_matched(&input).unwrap(),
        tapas::filters::FilterOutput::new(
            expected.as_bytes().to_vec(),
            EvidenceClass::PotentiallyLossy
        )
    );
}

#[test]
fn argv_diff_dispatch_matches_the_pinned_oracle_and_bypasses_summary_modes() {
    let input = fixture("git_diff_simple.txt");
    let expected = b"d simple.txt\n@1|1,3\n line one\n+line two\n+line three\n";

    assert_eq!(
        git::dispatch_argv(&[b"git", b"diff"], &input, b"ignored stderr", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(expected.to_vec(), EvidenceClass::FactComplete)
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"diff", b"--stat"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(input, EvidenceClass::ByteExact)
    );
}

#[test]
fn argv_status_dispatch_matches_long_and_short_oracle_output() {
    let dirty = fixture("git_status_dirty.txt");
    let expected_dirty = b"# main =origin/main\nM src/main.zig\nM src/pipeline.zig\n? src/filters/git_status.zig\n? tests/fixtures/git_status_dirty.txt\n";
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status"], &dirty, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(expected_dirty.to_vec(), EvidenceClass::FactComplete)
    );

    let short = fixture("git_status_short.txt");
    let expected_short = concat!(
        " M src/filters/git_status.zig\n",
        " M git_log.zig\n",
        " M git_diff.zig\n",
        " M src/main.zig\n",
        "M  src/pipeline.zig\n",
        "A  src/filters/git_reflog.zig\n",
        "?? tests/fixtures/git_status_short.txt\n",
        "?? git_reflog.txt\n",
        "?? git_tag.txt\n",
        "R  src/old.zig -> src/new.zig\n",
        "UU src/conflict.zig\n",
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status", b"-s"], &short, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            expected_short.as_bytes().to_vec(),
            EvidenceClass::FactComplete,
        )
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status", b"--porcelain"], &short, b"", 0, false,).unwrap(),
        tapas::filters::FilterOutput::new(short, EvidenceClass::ByteExact)
    );
}

#[test]
fn argv_log_dispatch_matches_oracle_and_honors_custom_format_bypasses() {
    let input = fixture("git_log_linear.txt");
    let expected =
        b"f0ad49e fix: third line\nf666a84 feat: extend a.txt\n95cbeda feat: add a.txt with one line\n";
    assert_eq!(
        git::dispatch_argv(&[b"git", b"log"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(expected.to_vec(), EvidenceClass::PotentiallyLossy)
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"log", b"--graph"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(input, EvidenceClass::ByteExact)
    );
}

#[test]
fn argv_log_stat_dispatch_matches_the_pinned_oracle() {
    let input = fixture("git_log_stat.txt");
    let expected = concat!(
        "abcdef0 round 8 updates [Refs: SMLL-42; Fixes: #123]\n",
        "  docs/guides/release_08.md      |  2 ++\n",
        "  src/core/ (6 files, +30 -0)\n",
        "  src/ui/ (6 files, +30 -0)\n",
        "  src/{old_name.zig => name.zig} | 10 +++++-----\n",
        "  14 files changed, 67 insertions(+), 5 deletions(-)\n",
        "b2c3d4e fix: keep count summaries [BREAKING CHANGE: count lines stay visible]\n",
        "  src/filters/git_log.zig | 18 ++++++++++++------\n",
        "  src/wrapper_git.zig     |  9 +++++----\n",
        "  2 files changed, 17 insertions(+), 10 deletions(-)\n",
    );

    assert_eq!(
        git::dispatch_argv(&[b"git", b"log", b"--stat"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            expected.as_bytes().to_vec(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn argv_show_dispatch_matches_oracle_and_preserves_blob_specs() {
    let input = fixture("git_show_body.txt");
    let expected = b"f666a84 feat: extend a.txt\n  This body explains why we added a second line.\n  It spans multiple lines and contains punctuation.\n\nd a.txt\n@1|1,2\n line1\n+line2\n";
    assert_eq!(
        git::dispatch_argv(&[b"git", b"show"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(expected.to_vec(), EvidenceClass::PotentiallyLossy)
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"show", b"HEAD:file"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(input, EvidenceClass::ByteExact)
    );
}

#[test]
fn argv_branch_dispatch_matches_the_pinned_oracle() {
    let input = fixture("git_branch_list.txt");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"branch"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b" feature-x\n feature-y\n* main\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn argv_release_and_audit_commands_keep_their_actionable_rows() {
    let tags = fixture("git_tag_list.txt");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"tag", b"--list"], &tags, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b"v0.1.0\nv0.1.0-rc.1\nv0.2.0\nv0.3.0\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    let worktrees = fixture("git_worktree_list.txt");
    let worktree_output =
        git::dispatch_argv(&[b"git", b"worktree", b"list"], &worktrees, b"", 0, false).unwrap();
    assert!(
        worktree_output
            .bytes
            .starts_with(b"/repo/tapas 1e94d59c6b5f8a1e2b3c4d5e6f708192a3b4c5d6 [main]\n")
    );
    assert!(
        worktree_output
            .bytes
            .windows(b"[review/catalog]".len())
            .any(|window| window == b"[review/catalog]")
    );

    let shortlog = fixture("git_shortlog.txt");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"shortlog", b"-sne"], &shortlog, b"", 0, false)
            .unwrap()
            .bytes,
        b"12 Niels Kootstra\n5 Release Bot\n2 Tapas Contributors\n"
    );

    let remotes = fixture("git_remote_verbose.txt");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"remote", b"-v"], &remotes, b"", 0, false)
            .unwrap()
            .bytes,
        b"origin git@github.com:nkootstra/tapas.git (fetch)\norigin git@github.com:nkootstra/tapas.git (push)\nupstream https://github.com/example/tapas.git (fetch)\n"
    );

    let config = fixture("git_config_list.txt");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"config", b"--list"], &config, b"", 0, false)
            .unwrap()
            .bytes,
        b"user.name=Niels Kootstra\nuser.email=niels@example.com\ngpg.format=ssh\ntag.gpgSign=true\n"
    );
}

#[test]
fn git_machine_formats_remain_byte_exact() {
    let tags = fixture("git_tag_list.txt");
    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"tag", b"--format=%(refname)"],
            &tags,
            b"",
            0,
            false
        )
        .unwrap(),
        tapas::filters::FilterOutput::new(tags, EvidenceClass::ByteExact)
    );

    let worktrees = fixture("git_worktree_list.txt");
    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"worktree", b"list", b"--porcelain"],
            &worktrees,
            b"",
            0,
            false
        )
        .unwrap(),
        tapas::filters::FilterOutput::new(worktrees, EvidenceClass::ByteExact)
    );

    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"config", b"--get", b"foo.empty"],
            b"\n",
            b"",
            0,
            false,
        )
        .unwrap(),
        tapas::filters::FilterOutput::new(b"\n".to_vec(), EvidenceClass::ByteExact)
    );
}

#[test]
fn argv_reflog_dispatch_uses_shape_guard_and_custom_format_bypass() {
    let input = fixture("git_reflog.txt");
    let oracle = git::apply_matched(&input).unwrap();
    assert_eq!(
        git::dispatch_argv(&[b"git", b"reflog"], &input, b"", 0, false).unwrap(),
        oracle
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"reflog", b"--format=%H"], &input, b"", 0, false,).unwrap(),
        tapas::filters::FilterOutput::new(input, EvidenceClass::ByteExact)
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"reflog"], b"unexpected\n", b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(b"unexpected\n".to_vec(), EvidenceClass::ByteExact)
    );
}

#[test]
fn argv_commit_dispatch_matches_the_pinned_oracle() {
    let input = fixture("git_commit_simple.txt");

    assert_eq!(
        git::dispatch_argv(&[b"git", b"commit"], &input, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b"c 6b68b6d main feat: add a.txt\n+1/-0 files=1\n+ a.txt\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn argv_merge_dispatch_matches_the_pinned_oracle_and_failed_commands_are_byte_exact() {
    let success = fixture("git_merge_ff.txt");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"merge", b"feature-x"], &success, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b"@ ff 81a7b77..af90dc8\nfx.txt |1\n+1/-0 files=1\n+ fx.txt\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    let failed = fixture("git_merge_conflict.stdout.txt");
    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"merge", b"conflicting"],
            &failed,
            b"diagnostic stderr\n",
            1,
            false,
        )
        .unwrap(),
        tapas::filters::FilterOutput::new(failed, EvidenceClass::ByteExact)
    );
}

#[test]
fn argv_blame_dispatch_matches_the_pinned_oracle_and_bypasses_alternate_formats() {
    let input = fixture("git_blame_simple.txt");
    let oracle = git::apply_matched(&input).unwrap();

    assert_eq!(
        git::dispatch_argv(&[b"git", b"blame", b"src/main.zig"], &input, b"", 0, false).unwrap(),
        oracle
    );
    for alternative in [
        b"-s".as_slice(),
        b"--porcelain",
        b"-p",
        b"--line-porcelain",
        b"--incremental",
        b"-e",
        b"--show-email",
    ] {
        assert_eq!(
            git::dispatch_argv(
                &[b"git", b"blame", alternative, b"src/main.zig"],
                &input,
                b"",
                0,
                false,
            )
            .unwrap(),
            tapas::filters::FilterOutput::new(input.clone(), EvidenceClass::ByteExact)
        );
    }
}

#[test]
fn stream_dispatch_matches_pinned_argv_only_command_helpers() {
    let stderr_cases: &[(&[u8], &str, &str, &[u8])] = &[
        (
            b"add",
            "git_add_error.stdout.txt",
            "git_add_error.stderr.txt",
            b"! nonexistent-path\n",
        ),
        (
            b"checkout",
            "git_checkout_switch.stdout.txt",
            "git_checkout_switch.stderr.txt",
            b"^ feature-x\n",
        ),
        (
            b"fetch",
            "git_fetch_simple.stdout.txt",
            "git_fetch_simple.stderr.txt",
            b"< 2cee6f5..81a7b77 main       -> origin/main\n",
        ),
    ];
    for &(subcommand, stdout_fixture, stderr_fixture, expected) in stderr_cases {
        let output = git::dispatch_streams_argv(
            &[b"git", subcommand],
            &fixture(stdout_fixture),
            &fixture(stderr_fixture),
            0,
            false,
        )
        .unwrap();
        assert!(output.stdout.is_empty(), "subcommand {:?}", subcommand);
        assert_eq!(output.stderr, expected, "subcommand {:?}", subcommand);
        assert_eq!(output.evidence, EvidenceClass::FactComplete);
    }

    let stdout_cases: &[(&[u8], &str, &str, &[u8])] = &[
        (
            b"rebase",
            "git_rebase_simple.txt",
            "git_add_error.stdout.txt",
            b"@ rebased rebase-branch\n",
        ),
        (
            b"stash",
            "git_stash_list.txt",
            "git_add_error.stdout.txt",
            b"$0 main wip: fixture stash entry 2\n$1 main wip: fixture stash entry 1\n",
        ),
    ];
    for &(subcommand, stdout_fixture, stderr_fixture, expected) in stdout_cases {
        let output = git::dispatch_streams_argv(
            &[b"git", subcommand],
            &fixture(stdout_fixture),
            &fixture(stderr_fixture),
            0,
            false,
        )
        .unwrap();
        assert_eq!(output.stdout, expected, "subcommand {:?}", subcommand);
        assert!(output.stderr.is_empty(), "subcommand {:?}", subcommand);
        assert_eq!(output.evidence, EvidenceClass::FactComplete);
    }

    assert_eq!(
        git::dispatch_streams_argv(
            &[b"git", b"pull"],
            &fixture("git_pull_ff.stdout.txt"),
            &fixture("git_pull_ff.stderr.txt"),
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"@ fast-forward 43fe7da..2cee6f5\n+1/-0 files=1\n".to_vec(),
            b"< 43fe7da..2cee6f5 main       -> origin/main\n".to_vec(),
            EvidenceClass::FactComplete,
        ),
    );
    assert_eq!(
        git::dispatch_streams_argv(
            &[b"git", b"push"],
            &fixture("git_push_simple.stdout.txt"),
            &fixture("git_push_simple.stderr.txt"),
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            Vec::new(),
            b"+ new main -> main\n".to_vec(),
            EvidenceClass::FactComplete,
        ),
    );
}

#[test]
fn pull_and_push_fail_open_only_on_the_unrecognized_source_stream() {
    let pull_stdout = fixture("git_pull_ff.stdout.txt");
    let pull_stderr = fixture("git_pull_ff.stderr.txt");
    let unexpected_stdout = b"server supplied an unexpected success notice\n";
    assert_eq!(
        git::dispatch_streams_argv(
            &[b"git", b"pull"],
            unexpected_stdout,
            &pull_stderr,
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            unexpected_stdout.to_vec(),
            b"< 43fe7da..2cee6f5 main       -> origin/main\n".to_vec(),
            EvidenceClass::FactComplete,
        ),
    );

    let unknown_stderr = b"remote helper supplied an unexpected success notice\n";
    assert_eq!(
        git::dispatch_streams_argv(&[b"git", b"pull"], &pull_stdout, unknown_stderr, 0, false,)
            .unwrap(),
        StreamFilterOutput::new(
            b"@ fast-forward 43fe7da..2cee6f5\n+1/-0 files=1\n".to_vec(),
            unknown_stderr.to_vec(),
            EvidenceClass::FactComplete,
        ),
    );

    let push_stderr = fixture("git_push_simple.stderr.txt");
    assert_eq!(
        git::dispatch_streams_argv(
            &[b"git", b"push"],
            b"unexpected stdout\n",
            &push_stderr,
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"unexpected stdout\n".to_vec(),
            b"+ new main -> main\n".to_vec(),
            EvidenceClass::FactComplete,
        ),
    );
}

#[test]
fn pull_and_push_preserve_unsafe_variants_byte_exact() {
    let invalid = b"branch 'main' set up to track 'origin/main'.\n\xff";
    let malformed = b"branch 'main' set up to track origin/main.\n";
    let stderr = fixture("git_push_simple.stderr.txt");
    for stdout in [invalid.as_slice(), malformed.as_slice()] {
        let output =
            git::dispatch_streams_argv(&[b"git", b"push"], stdout, &stderr, 0, false).unwrap();
        assert_eq!(output.stdout, stdout);
        assert_eq!(output.stderr, b"+ new main -> main\n");
    }

    for (exit_code, lossless) in [(1, false), (0, true)] {
        let stdout = fixture("git_pull_ff.stdout.txt");
        let stderr = fixture("git_pull_ff.stderr.txt");
        assert_eq!(
            git::dispatch_streams_argv(&[b"git", b"pull"], &stdout, &stderr, exit_code, lossless,)
                .unwrap(),
            StreamFilterOutput::passthrough(&stdout, &stderr),
        );
    }
}

#[test]
fn push_suppresses_each_recognized_tracking_boilerplate_line() {
    let stdout = b"branch 'main' set up to track 'origin/main'.\n\
branch 'feature' set up to track 'origin/feature'.\n";
    assert_eq!(
        git::dispatch_streams_argv(&[b"git", b"push"], stdout, b"", 0, false).unwrap(),
        StreamFilterOutput::new(Vec::new(), Vec::new(), EvidenceClass::FactComplete),
    );
}

#[test]
fn stdout_only_dispatch_compacts_argv_only_helpers_without_owning_stderr() {
    let warning = b"warning: CRLF will be replaced by LF in src/main.rs.\n";
    assert_eq!(
        git::dispatch_argv(&[b"git", b"add"], warning, b"leave stderr alone", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(b"! src/main.rs\n".to_vec(), EvidenceClass::FactComplete,)
    );

    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"switch", b"main"],
            b"M\tsrc/main.rs\nD\told.rs\n",
            b"Switched to branch 'main'\n",
            0,
            false,
        )
        .unwrap(),
        tapas::filters::FilterOutput::new(
            b"M src/main.rs\nd old.rs\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    let pull = fixture("git_pull_ff.stdout.txt");
    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"pull"],
            &pull,
            &fixture("git_pull_ff.stderr.txt"),
            0,
            false,
        )
        .unwrap(),
        tapas::filters::FilterOutput::new(
            b"@ fast-forward 43fe7da..2cee6f5\n+1/-0 files=1\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"rebase"],
            &fixture("git_rebase_simple.txt"),
            b"",
            0,
            false,
        )
        .unwrap()
        .bytes,
        b"@ rebased rebase-branch\n"
    );
    assert_eq!(
        git::dispatch_argv(
            &[b"git", b"stash", b"list"],
            &fixture("git_stash_list.txt"),
            b"",
            0,
            false,
        )
        .unwrap()
        .bytes,
        b"$0 main wip: fixture stash entry 2\n$1 main wip: fixture stash entry 1\n"
    );
    assert!(
        git::dispatch_argv(
            &[b"git", b"push"],
            &fixture("git_push_simple.stdout.txt"),
            &fixture("git_push_simple.stderr.txt"),
            0,
            false,
        )
        .unwrap()
        .bytes
        .is_empty()
    );
}
