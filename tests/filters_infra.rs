use tapas::filters::{EvidenceClass, StreamFilterOutput, infra};

const FIXTURES: &str = "regression/fixtures";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/{FIXTURES}/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn docker_ps_and_images_match_the_pinned_compact_grammars() {
    let ps = fixture("docker_ps.txt");
    assert_eq!(
        infra::dispatch_streams_argv(&[b"docker", b"ps"], &ps, b"", 0, false)
            .unwrap()
            .stdout,
        b"d4up helios-assistant helios-convex-dashboard helios-convex-backend helios-mysql\n",
    );

    let compose = fixture("docker_compose_ps.txt");
    assert_eq!(
        infra::dispatch_streams_argv(&[b"docker", b"compose", b"ps"], &compose, b"", 0, false,)
            .unwrap()
            .stdout,
        b"d1up smll_d4_fixture-echoer-1(node:24-alpine)\n",
    );

    let images = fixture("docker_images.txt");
    let output =
        infra::dispatch_streams_argv(&[b"docker", b"images"], &images, b"", 0, false).unwrap();
    assert!(contains(&output.stdout, b"images 25:"));
    assert!(contains(&output.stdout, b"postgres:18(479MB)"));
    assert!(contains(&output.stdout, b"dangling x3"));
    assert!(contains(&output.stdout, b"(+14)"));
}

#[test]
fn docker_and_kubectl_logs_match_the_pinned_oracle() {
    let input = fixture("docker_logs.txt");
    let expected = concat!(
        "INFO  starting server on :8080\n",
        "INFO  connected to postgres\n",
        "INFO  GET /health 200 2ms ×5\n",
        "WARN  slow query: SELECT * FROM users WHERE id=123 (512ms)\n",
        "INFO  GET /api/users 200 45ms ×3\n",
        "ERROR failed to connect to redis: connection refused ×4\n",
        "INFO  redis reconnected\n",
        "INFO  GET /health 200 1ms ×4\n",
        "INFO  shutting down gracefully\n",
    );
    for argv in [
        &[b"docker".as_slice(), b"logs", b"app"][..],
        &[b"kubectl".as_slice(), b"logs", b"app"][..],
    ] {
        assert_eq!(
            infra::dispatch_streams_argv(argv, &input, b"", 0, false)
                .unwrap()
                .stdout,
            expected.as_bytes(),
        );
    }

    let compose = fixture("docker_compose_logs.txt");
    assert_eq!(
        infra::dispatch_streams_argv(
            &[b"docker", b"compose", b"logs", b"app"],
            &compose,
            b"",
            0,
            false,
        )
        .unwrap()
        .stdout,
        "echoer-1| ready ×3\nechoer-1| done\n".as_bytes(),
    );
}

#[test]
fn kubectl_pods_fixture_matches_the_pinned_oracle() {
    let input = fixture("kubectl_pods.txt");
    let output =
        infra::dispatch_streams_argv(&[b"kubectl", b"get", b"pods"], &input, b"", 0, false)
            .unwrap();
    assert!(output.stdout.starts_with(b"k9r "));
    assert!(contains(&output.stdout, b"api-server-6f8b9c4d7-x2k8m"));
    assert!(contains(&output.stdout, b"redis-master-0"));
    assert!(!output.stdout.contains(&b'('));
}

#[test]
fn verbose_curl_with_body_and_trace_preserves_both_descriptors() {
    let stdout = fixture("curl_v_example.stdout.txt");
    let stderr = fixture("curl_v_example.stderr.txt");
    let output = infra::dispatch_streams_argv(
        &[b"curl", b"-v", b"https://example.com"],
        &stdout,
        &stderr,
        0,
        false,
    )
    .unwrap();
    assert_eq!(output.stdout, stdout);
    assert!(contains(&output.stderr, b"HTTP/2 200"));
    assert!(!contains(&output.stderr, b"TLSv1.3"));
    assert!(output.stderr.len() < stderr.len());
}

#[test]
fn repeated_verbose_curl_compacts_trace_without_moving_the_body() {
    let stdout = fixture("large/curl_vvv_example.stdout.txt");
    let stderr = fixture("large/curl_vvv_example.stderr.txt");
    let output = infra::dispatch_streams_argv(
        &[b"curl", b"-vvv", b"https://api.example.com/v1/resources"],
        &stdout,
        &stderr,
        0,
        false,
    )
    .unwrap();
    assert_eq!(output.stdout, stdout);
    assert!(contains(&output.stderr, b"HTTP/2 200"));
    assert!(output.stderr.len() < stderr.len());
}

#[test]
fn gh_list_routes_match_the_pinned_oracle_shapes() {
    let prs = fixture("gh_pr_list.txt");
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"pr", b"list"], &prs, b"", 0, false).unwrap();
    assert!(
        output
            .stdout
            .starts_with(b"Showing 8 of 8 open pull requests")
    );
    assert!(!contains(&output.stdout, b"ID   TITLE"));
    assert!(contains(
        &output.stdout,
        b"#142 feat: Add user authentication flow"
    ));

    let runs = fixture("gh_run_list.txt");
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"run", b"list"], &runs, b"", 0, false).unwrap();
    assert!(
        output
            .stdout
            .starts_with(b"STATUS TITLE WORKFLOW BRANCH EVENT ID ELAPSED AGE\n")
    );
    assert!(contains(
        &output.stdout,
        b"failure release size gate Release main workflow_run"
    ));
}

#[test]
fn gh_extended_list_and_json_routes_keep_actionable_facts() {
    let search = fixture("gh_search_repos.txt");
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"search", b"repos"], &search, b"", 0, false)
            .unwrap();
    assert!(contains(
        &output.stdout,
        b"cli/cli\tGitHub's official command-line tool"
    ));
    assert!(contains(
        &output.stdout,
        b"sharkdp/fd\tA simple, fast alternative to find"
    ));
    assert!(!contains(&output.stdout, b"                       "));

    let empty_field = b"A  B  C\n1     3\n";
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"search", b"repos"], empty_field, b"", 0, false)
            .unwrap();
    assert_eq!(output.stdout, b"A\tB\tC\n1\t\t3\n");

    let releases = fixture("gh_release_list.txt");
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"release", b"list"], &releases, b"", 0, false)
            .unwrap();
    assert!(contains(&output.stdout, b"Tapas 0.2.0\tLatest\tv0.2.0"));
    assert!(contains(&output.stdout, b"v0.1.0-rc.1"));

    let issues = fixture("gh_issue_list.txt");
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"issue", b"list"], &issues, b"", 0, false).unwrap();
    assert!(contains(
        &output.stdout,
        b"Preserve actionable output for gh search\t#42\tOPEN\tnkootstra"
    ));
    assert!(contains(&output.stdout, b"#41\tOPEN\tagent"));
}

#[test]
fn gh_api_and_json_selection_compact_json_but_jq_stays_exact() {
    let json = fixture("gh_api.json");
    let expected =
        b"{\"nameWithOwner\":\"nkootstra/tapas\",\"description\":\"Compact command output for coding agents\",\"isPrivate\":false,\"defaultBranchRef\":{\"name\":\"main\"},\"topics\":[\"agents\",\"cli\",\"rust\"]}\n";

    for argv in [
        &[b"gh".as_slice(), b"api", b"repos/nkootstra/tapas"][..],
        &[
            b"gh".as_slice(),
            b"repo",
            b"view",
            b"--json",
            b"nameWithOwner",
        ][..],
    ] {
        let output = infra::dispatch_streams_argv(argv, &json, b"", 0, false).unwrap();
        assert_eq!(output.stdout, expected);
    }

    let jq_output = b"nkootstra/tapas\n";
    let output = infra::dispatch_streams_argv(
        &[
            b"gh",
            b"api",
            b"repos/nkootstra/tapas",
            b"--jq",
            b".nameWithOwner",
        ],
        jq_output,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(output.stdout, jq_output);
}

#[test]
fn gh_detail_routes_preserve_failures_and_fold_repetition() {
    let view = fixture("gh_pr_view.txt");
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"pr", b"view", b"67"], &view, b"", 0, false)
            .unwrap();
    assert!(output.stdout.starts_with(b"#67 MERGED feat(wrapper):"));
    assert!(contains(&output.stdout, b"author:\tnkootstra"));
    assert!(!contains(&output.stdout, b"labels:\t"));
    assert!(contains(&output.stdout, b"## What"));

    let checks = fixture("gh_pr_checks_pending.txt");
    let output =
        infra::dispatch_streams_argv(&[b"gh", b"pr", b"checks", b"67"], &checks, b"", 8, false)
            .unwrap();
    assert!(output.stdout.starts_with(b"6 checks: 3 pass, 3 pending\n"));
    assert!(contains(&output.stdout, b"Greptile Review\tpending"));
    assert!(!contains(&output.stdout, b"fmt-check\tpass"));

    let run = fixture("gh_run_view_failed.txt");
    let output = infra::dispatch_streams_argv(
        &[b"gh", b"run", b"view", b"27368462904"],
        &run,
        b"",
        1,
        false,
    )
    .unwrap();
    assert!(contains(&output.stdout, "JOBS\n✓ 3 passed\n".as_bytes()));
    assert!(contains(&output.stdout, "  ✓ 7 steps passed\n".as_bytes()));
    assert!(contains(&output.stdout, b"X Enforce release size cap"));
    assert!(contains(
        &output.stdout,
        b"X binary size 328152 exceeds release cap 327680"
    ));
    assert_eq!(
        output
            .stdout
            .windows(b"Node.js 20 actions are deprecated".len())
            .filter(|window| *window == b"Node.js 20 actions are deprecated")
            .count(),
        1,
    );
}

#[test]
fn acli_table_and_view_fixtures_match_the_pinned_contract() {
    let search = fixture("acli_jira_workitem_search.txt");
    let output = infra::dispatch_streams_argv(
        &[
            b"acli",
            b"jira",
            b"workitem",
            b"search",
            b"--jql",
            b"project = EXAMPLE",
        ],
        &search,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(
        output.stdout,
        concat!(
            "TYPE KEY ASSIGNEE PRIORITY STATUS SUMMARY\n",
            "Task EXAMPLE-101 Ada Example Medium In Progress Anonymized sample work item\n",
            "Bug EXAMPLE-102 Unassigned High To Do Consectetur adipiscing elit sed\n",
            "Story EXAMPLE-103 Linus Example Low Done Eiusmod tempor incididunt ut labore\n",
            "Sub-task EXAMPLE-104 Grace Example Medium In Review Ut enim ad minim veniam\n",
        )
        .as_bytes(),
    );

    let page = fixture("acli_confluence_page_view.txt");
    let output = infra::dispatch_streams_argv(
        &[b"acli", b"confluence", b"page", b"view"],
        &page,
        b"",
        0,
        false,
    )
    .unwrap();
    assert!(contains(&output.stdout, b"ID: 100000001"));
    assert!(contains(
        &output.stdout,
        b"Body:\n  Anonymized sample page body"
    ));
    assert!(contains(
        &output.stdout,
        b"Labels: lorem, ipsum, anonymized"
    ));
    assert!(!contains(&output.stdout, b"Duis aute irure"));
}

#[test]
fn acli_json_is_minified_without_changing_its_data() {
    let input = fixture("acli_jira_workitem_search_json.txt");
    let output = infra::dispatch_streams_argv(
        &[b"acli", b"jira", b"workitem", b"search", b"--json"],
        &input,
        b"",
        0,
        false,
    )
    .unwrap();

    assert_eq!(
        output.stdout,
        b"[{\"key\":\"EXAMPLE-101\",\"fields\":{\"summary\":\"Anonymized sample work item\",\"status\":{\"name\":\"In Progress\"},\"assignee\":{\"displayName\":\"Ada Example\"}}},{\"key\":\"EXAMPLE-102\",\"fields\":{\"summary\":\"Consectetur adipiscing elit sed\",\"status\":{\"name\":\"To Do\"},\"assignee\":null}}]\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn exact_lossless_failures_and_unknown_shapes_are_byte_exact() {
    let stdout = b"raw \xff\n";
    let stderr = b"err \xfe\n";
    for (argv, exit_code, lossless) in [
        (
            &[b"kubectl".as_slice(), b"get", b"pods", b"-ojson"][..],
            0,
            false,
        ),
        (
            &[b"docker".as_slice(), b"ps", b"--format", b"{{.ID}}"][..],
            0,
            false,
        ),
        (&[b"gh".as_slice(), b"pr", b"list"][..], 1, false),
        (
            &[b"acli".as_slice(), b"jira", b"workitem", b"search"][..],
            1,
            false,
        ),
        (&[b"curl".as_slice(), b"-v", b"url"][..], 0, true),
    ] {
        assert_eq!(
            infra::dispatch_streams_argv(argv, stdout, stderr, exit_code, lossless).unwrap(),
            StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact,),
        );
    }
    assert!(infra::dispatch_streams_argv(&[], stdout, stderr, 0, false).is_err());
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
