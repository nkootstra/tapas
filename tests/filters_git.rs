use tapas::filters::{EvidenceClass, StreamFilterOutput, git};
mod common;
use common::fixture;

type RouteCase<'a> = (&'a [&'a [u8]], &'a [u8], i32, bool);

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

    let clean = fixture("git_status_clean.txt");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status"], &clean, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b"# main =origin/main (clean)\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn argv_status_dispatch_marks_a_clean_tree_across_upstream_shapes() {
    // A dirty tree opens with the same branch line a clean one does, so without a marker
    // "clean" and "header emitted, entries lost" are byte-indistinguishable to a reader.
    let cases: &[(&[u8], &str)] = &[
        (
            b"On branch main\n\nnothing to commit, working tree clean\n",
            "# main (clean)\n",
        ),
        (
            concat!(
                "On branch main\n",
                "Your branch is ahead of 'origin/main' by 2 commits.\n",
                "  (use \"git push\" to publish your local commits)\n",
                "\nnothing to commit, working tree clean\n",
            )
            .as_bytes(),
            // write_branch_line suppresses the upstream whenever ahead or behind is set.
            "# main +2 (clean)\n",
        ),
        (
            concat!(
                "On branch main\n",
                "Your branch is behind 'origin/main' by 3 commits, and can be fast-forwarded.\n",
                "\nnothing to commit, working tree clean\n",
            )
            .as_bytes(),
            "# main -3 (clean)\n",
        ),
        (
            concat!(
                "On branch main\n",
                "Your branch and 'origin/main' have diverged,\n",
                "and have 1 and 2 different commits each, respectively.\n",
                "\nnothing to commit, working tree clean\n",
            )
            .as_bytes(),
            "# main +1 -2 (clean)\n",
        ),
        (
            b"HEAD detached at abc1234\n\nnothing to commit, working tree clean\n",
            "# HEAD:abc1234 (clean)\n",
        ),
        (
            // Unrecognized lines are dropped, but git still declared the tree clean, so
            // the marker rests on that claim rather than on the parse succeeding.
            concat!(
                "On branch main\n",
                "Your branch is up to date with 'origin/main'.\n",
                "\nIt took 2.00 seconds to enumerate untracked files.\n",
                "\nnothing to commit, working tree clean\n",
            )
            .as_bytes(),
            "# main =origin/main (clean)\n",
        ),
    ];

    for (input, expected) in cases {
        assert_eq!(
            git::dispatch_argv(&[b"git", b"status"], input, b"", 0, false).unwrap(),
            tapas::filters::FilterOutput::new(
                expected.as_bytes().to_vec(),
                EvidenceClass::FactComplete,
            ),
            "clean marker mismatch for {:?}",
            String::from_utf8_lossy(input),
        );
    }
}

#[test]
fn argv_status_dispatch_accepts_every_versioned_spelling_of_the_clean_sentence() {
    // Git spells this three ways across its history: `working tree` since 2.9,
    // `working directory` before it, and a parenthesized form before that.
    for sentence in [
        "nothing to commit, working tree clean",
        "nothing to commit, working directory clean",
        "nothing to commit (working directory clean)",
    ] {
        let input = format!("On branch main\n\n{sentence}\n");
        assert_eq!(
            git::dispatch_argv(&[b"git", b"status"], input.as_bytes(), b"", 0, false).unwrap(),
            tapas::filters::FilterOutput::new(
                b"# main (clean)\n".to_vec(),
                EvidenceClass::FactComplete,
            ),
            "clean sentence not recognized: {sentence}",
        );
    }

    // The parenthesized spelling is belt-and-braces rather than a reachable path: git old
    // enough to emit it prefixes every status line with `# `, which the filter declines,
    // so such a document is never compacted in the first place.
    let legacy = b"# On branch master\nnothing to commit (working directory clean)\n";
    assert!(!git::matches(legacy));
}

#[test]
fn argv_status_dispatch_reports_divergence_from_the_continuation_line() {
    // Git wraps divergence across two lines and keeps the counts on the continuation.
    // Reading only the opening line drops them, which understates a status report that
    // claims to be complete -- on a clean tree it would read as "nothing to report" while
    // the branch sits two commits behind.
    let diverged = concat!(
        "On branch main\n",
        "Your branch and 'origin/main' have diverged,\n",
        "and have 1 and 2 different commits each, respectively.\n",
        "  (use \"git pull\" if you want to integrate the remote branch with yours)\n",
    );

    let clean = format!("{diverged}\nnothing to commit, working tree clean\n");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status"], clean.as_bytes(), b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b"# main +1 -2 (clean)\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    // Independent of the clean marker: a dirty diverged tree reports the counts too.
    let dirty = format!("{diverged}\nChanges not staged for commit:\n\tmodified:   a.txt\n");
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status"], dirty.as_bytes(), b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b"# main +1 -2\nM a.txt\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    // Some versions keep the sentence on one line; that opening still parses.
    let inline = concat!(
        "On branch main\n",
        "Your branch and 'origin/main' have diverged, and have 4 and 5 different commits each, respectively.\n",
        "\nnothing to commit, working tree clean\n",
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status"], inline.as_bytes(), b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            b"# main +4 -5 (clean)\n".to_vec(),
            EvidenceClass::FactComplete,
        )
    );
}

#[test]
fn argv_status_dispatch_withholds_the_clean_marker_without_gits_own_claim() {
    // Cleanliness is never inferred from the absence of entries. Piped input can be cut
    // off mid-listing, and `git status` qualifies its own wording when the tree is only
    // conditionally clean -- asserting over either would be a false FactComplete.
    let cases: &[(&[u8], &str)] = &[
        (
            // `git status | head -5 | tapas`: unstaged changes exist, the section header
            // was parsed, and the entries never arrived.
            concat!(
                "On branch main\n",
                "Your branch is up to date with 'origin/main'.\n",
                "\nChanges not staged for commit:\n",
                "  (use \"git add <file>...\" to update what will be committed)\n",
            )
            .as_bytes(),
            "# main =origin/main\n",
        ),
        (
            // -uno: git says "nothing to commit" while untracked files exist.
            b"On branch main\n\nnothing to commit (use -u to show untracked files)\n",
            "# main\n",
        ),
        (
            // A repo with no commits yet.
            concat!(
                "On branch main\n",
                "\nNo commits yet\n",
                "\nnothing to commit (create/copy files and use \"git add\" to track)\n",
            )
            .as_bytes(),
            "# main\n",
        ),
    ];

    for (input, expected) in cases {
        assert_eq!(
            git::dispatch_argv(&[b"git", b"status"], input, b"", 0, false).unwrap(),
            tapas::filters::FilterOutput::new(
                expected.as_bytes().to_vec(),
                EvidenceClass::FactComplete,
            ),
            "unwarranted clean marker for {:?}",
            String::from_utf8_lossy(input),
        );
    }
}

#[test]
fn argv_status_dispatch_never_marks_an_operation_state_clean() {
    let conflict = fixture("git_status_conflict.txt");
    let expected_conflict = concat!(
        "# main\n",
        "! You have unmerged paths.\n",
        "S src/pipeline.zig\n",
        "UU src/filters/git_status.zig\n",
        "? tests/fixtures/git_status_conflict.txt\n",
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status"], &conflict, b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            expected_conflict.as_bytes().to_vec(),
            EvidenceClass::FactComplete,
        )
    );

    // Every state notice git prints above the listing, including the ones that do not
    // open with "You are currently ". A clean index does not mean the operation is done,
    // and dropping the notice would report outstanding work as nothing to report.
    for state in [
        "You are currently bisecting.",
        "You are currently cherry-picking commit abc1234.",
        "You are currently reverting commit abc1234.",
        "You are in the middle of an am session.",
        "The current patch is empty.",
        "Cherry-pick currently in progress.",
        "Revert currently in progress.",
        "You are editing the todo file of an ongoing interactive rebase.",
        "You are not currently on a branch.",
        "You are in a sparse checkout with 40% of tracked files present.",
    ] {
        let input =
            format!("On branch main\n{state}\n  (hint)\n\nnothing to commit, working tree clean\n");
        let expected = format!("# main\n! {state}\n");
        assert_eq!(
            git::dispatch_argv(&[b"git", b"status"], input.as_bytes(), b"", 0, false).unwrap(),
            tapas::filters::FilterOutput::new(expected.into_bytes(), EvidenceClass::FactComplete,),
            "operation state marked clean or dropped: {state}",
        );
    }

    // The index is clean mid-rebase, but the rebase itself is outstanding work.
    let rebase = concat!(
        "interactive rebase in progress; onto abc1234\n",
        "Last command done (1 command done):\n",
        "   pick 1234567 feat: a thing\n",
        "No commands remaining.\n",
        "You are currently rebasing branch 'topic' on 'abc1234'.\n",
        "  (all conflicts fixed: run \"git rebase --continue\")\n",
        "\nnothing to commit, working tree clean\n",
    );
    let expected_rebase = concat!(
        "# rebase-in-progress\n",
        "! interactive rebase in progress; onto abc1234\n",
        "Last command done (1 command done):\n",
        "   pick 1234567 feat: a thing\n",
        "No commands remaining.\n",
        "! You are currently rebasing branch 'topic' on 'abc1234'.\n",
    );
    assert_eq!(
        git::dispatch_argv(&[b"git", b"status"], rebase.as_bytes(), b"", 0, false).unwrap(),
        tapas::filters::FilterOutput::new(
            expected_rebase.as_bytes().to_vec(),
            EvidenceClass::FactComplete,
        )
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
fn porcelain_bypass_covers_the_valued_spellings() {
    // `--porcelain` also takes a value. Matching the bare flag alone let `--porcelain=v2`
    // reach the long-format parser, which recognizes none of it: every line was dropped,
    // the empty result tripped the no-output hint, and a tree with real changes reported
    // `(tapas: no changes; git status exited 0 with no output)`.
    let v2 = b"1 .M N... 100644 100644 100644 abc def src/a.rs\n? untracked.txt\n".to_vec();
    for form in [
        b"--porcelain".as_slice(),
        b"--porcelain=v1",
        b"--porcelain=v2",
    ] {
        assert_eq!(
            git::dispatch_argv(&[b"git", b"status", form], &v2, b"", 0, false).unwrap(),
            tapas::filters::FilterOutput::new(v2.clone(), EvidenceClass::ByteExact),
            "porcelain spelling not bypassed: {}",
            String::from_utf8_lossy(form),
        );
    }
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

#[test]
fn graphite_log_routes_compact_only_recognized_gt_graphs() {
    let default = concat!(
        "\u{25c9} feature/two (current)\n",
        "\u{2502} 8 seconds ago\n",
        "\u{2502}\n",
        "\u{2502} abcdef0 - second change\n",
        "\u{25cb} feature/one\n",
        "\u{2502} 2 minutes ago\n",
        "\u{2502}\n",
        "\u{2502} 1234567 - first change\n",
        "\u{25cb} main\n",
    );
    let expected = concat!(
        "\u{25c9} feature/two (current)\n",
        "\u{2502} abcdef0 - second change\n",
        "\u{25cb} feature/one\n",
        "\u{2502} 1234567 - first change\n",
        "\u{25cb} main\n",
    );
    assert_eq!(
        git::dispatch_streams_argv(&[b"graphite", b"log"], default.as_bytes(), b"", 0, false,)
            .unwrap()
            .stdout,
        expected.as_bytes(),
    );
    assert_eq!(
        git::dispatch_streams_argv(
            &[b"/opt/graphite/bin/gt", b"log"],
            default.as_bytes(),
            b"graphite notice\n",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            b"graphite notice\n".to_vec(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let short =
        "\u{25c9} feature/two\n\u{2502} \u{25cb} feature/one\n\u{25cb}\u{2500}\u{2518} main\n";
    assert_eq!(
        git::dispatch_streams_argv(&[b"gt", b"log", b"short"], short.as_bytes(), b"", 0, false,)
            .unwrap(),
        StreamFilterOutput::new(
            short.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let long = "* abcdef0 - (7 minutes ago) second change - Ada (feature/two)\n* 1234567 - (8 minutes ago) first change - Ada (main)\n";
    assert_eq!(
        git::dispatch_streams_argv(&[b"gt", b"log", b"long"], long.as_bytes(), b"", 0, false)
            .unwrap(),
        StreamFilterOutput::new(
            long.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn graphite_status_reuses_git_status_only_after_shape_validation() {
    let status = fixture("git_status_dirty.txt");
    assert_eq!(
        git::dispatch_streams_argv(
            &[b"gt", b"status"],
            &status,
            b"graphite notice\n",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"# main =origin/main\nM src/main.zig\nM src/pipeline.zig\n? src/filters/git_status.zig\n? tests/fixtures/git_status_dirty.txt\n".to_vec(),
            b"graphite notice\n".to_vec(),
            EvidenceClass::FactComplete,
        ),
    );
}

#[test]
fn graphite_unrecognized_alias_malformed_exact_and_failed_routes_are_byte_exact() {
    let graph = "\u{25c9} feature\n\u{25cb} main\n";
    let stderr = b"diagnostic \xff\n";
    let cases: &[RouteCase<'_>] = &[
        (&[b"graphite", b"log"], graph.as_bytes(), 0, false),
        (&[b"gt", b"ls"], graph.as_bytes(), 0, false),
        (&[b"gt", b"log", b"brief"], graph.as_bytes(), 0, false),
        (&[b"gt", b"log"], b"not a graph\n", 0, false),
        (&[b"gt", b"log"], b"\xe2\x97\x89 feature\n\xff", 0, false),
        (&[b"gt", b"log", b"--format=json"], b"{}\n", 0, false),
        (&[b"gt", b"log"], graph.as_bytes(), 1, false),
        (&[b"gt", b"log"], graph.as_bytes(), 0, true),
        (&[b"gt", b"status"], b"unexpected status\n", 0, false),
    ];
    for &(argv, stdout, exit_code, lossless) in cases {
        assert_eq!(
            git::dispatch_streams_argv(argv, stdout, stderr, exit_code, lossless).unwrap(),
            StreamFilterOutput::passthrough(stdout, stderr),
            "argv {argv:?}",
        );
    }
}
