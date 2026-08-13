use tapas::filters::{EvidenceClass, StreamFilterOutput, data};
mod common;
use common::fixture;

type StreamsCase<'a> = (&'a [&'a [u8]], &'a [u8], &'a [u8]);
type ExitCase<'a> = (&'a [&'a [u8]], &'a [u8], i32);

#[test]
fn structured_json_dispatch_minifies_single_containers_byte_safely() {
    let input =
        b"\xef\xbb\xbf{\n  \"a\": \"x y\",\n  \"b\": [1, 2],\n  \"raw\": \"}\\\\\\\"{\"\n}\r\n";
    let expected = b"{\"a\":\"x y\",\"b\":[1,2],\"raw\":\"}\\\\\\\"{\"}\n";

    for argv in [
        &[
            b"gh".as_slice(),
            b"issue",
            b"list",
            b"--json",
            b"number,title",
        ][..],
        &[b"acli".as_slice(), b"jira", b"workitem", b"search"][..],
    ] {
        assert_eq!(
            data::dispatch_streams_argv(argv, input, b"", 0, false).unwrap(),
            StreamFilterOutput::new(
                expected.to_vec(),
                Vec::new(),
                EvidenceClass::PotentiallyLossy,
            ),
            "argv {argv:?}",
        );
    }

    assert_eq!(
        data::dispatch_streams_argv(&[b"jq", b"."], input, b"", 0, false).unwrap(),
        StreamFilterOutput::new(input.to_vec(), Vec::new(), EvidenceClass::ByteExact),
    );
}

#[test]
fn aws_only_compacts_explicit_recognized_table_output() {
    let table = concat!(
        "-----------------------\n",
        "|   DescribeRegions   |\n",
        "+----------+-----------+\n",
        "| Name     | Endpoint  |\n",
        "+----------+-----------+\n",
        "| eu-west-1| example-1 |\n",
        "| eu-west-2| example-2 |\n",
        "+----------+-----------+\n",
    );
    let compact = data::dispatch_streams_argv(
        &[b"aws", b"ec2", b"describe-regions", b"--output", b"table"],
        table.as_bytes(),
        b"warning\n",
        0,
        false,
    )
    .unwrap();
    assert_eq!(
        compact,
        StreamFilterOutput::new(
            b"DescribeRegions\nName\tEndpoint\neu-west-1\texample-1\neu-west-2\texample-2\n"
                .to_vec(),
            b"warning\n".to_vec(),
            EvidenceClass::PotentiallyLossy,
        )
    );

    let json = b"{\n  \"Regions\": []\n}\n";
    for argv in [
        &[b"aws".as_slice(), b"ec2", b"describe-regions"][..],
        &[
            b"aws".as_slice(),
            b"ec2",
            b"describe-regions",
            b"--output=json",
        ][..],
        &[b"aws".as_slice(), b"ec2", b"describe-regions", b"--query=x"][..],
    ] {
        assert_eq!(
            data::dispatch_streams_argv(argv, json, b"", 0, false).unwrap(),
            StreamFilterOutput::new(json.to_vec(), Vec::new(), EvidenceClass::ByteExact),
            "argv {argv:?}",
        );
    }

    for malformed in [
        b"Name   Endpoint\neu-west-1   example-1\n".as_slice(),
        b"+---+---+\n| A | B |\n+---+---+\n| 1 | 2 | 3 |\n+---+---+\n",
        b"+---+\n| Name \xff |\n+---+\n".as_slice(),
    ] {
        assert_eq!(
            data::dispatch_streams_argv(
                &[b"aws", b"ec2", b"describe-regions", b"--output=table"],
                malformed,
                b"",
                0,
                false,
            )
            .unwrap(),
            StreamFilterOutput::new(malformed.to_vec(), Vec::new(), EvidenceClass::ByteExact),
        );
    }
}

#[test]
fn psql_only_compacts_finite_aligned_table_routes() {
    let table = b" id | name  \n----+-------\n 1  | alpha \n 2  | beta  \n(2 rows)\n";
    for argv in [
        &[b"psql".as_slice(), b"-c", b"select id, name from items"][..],
        &[b"psql".as_slice(), b"--command=select id, name from items"][..],
        &[b"psql".as_slice(), b"--list"][..],
        &[b"psql".as_slice(), b"-l"][..],
    ] {
        assert_eq!(
            data::dispatch_streams_argv(argv, table, b"", 0, false).unwrap(),
            StreamFilterOutput::new(
                b"id\tname\n1\talpha\n2\tbeta\n(2 rows)\n".to_vec(),
                Vec::new(),
                EvidenceClass::PotentiallyLossy,
            ),
            "argv {argv:?}",
        );
    }

    for argv in [
        &[b"psql".as_slice(), b"database"][..],
        &[b"psql".as_slice(), b"--unknown"][..],
        &[b"psql".as_slice(), b"-c", b"select 1", b"database"][..],
        &[b"psql".as_slice(), b"-At", b"-c", b"select 1"][..],
        &[
            b"psql".as_slice(),
            b"--output",
            b"rows.txt",
            b"-c",
            b"select 1",
        ][..],
        &[
            b"psql".as_slice(),
            b"-c",
            b"\\copy items to stdout with csv",
        ][..],
    ] {
        assert_eq!(
            data::dispatch_streams_argv(argv, table, b"", 0, false).unwrap(),
            StreamFilterOutput::new(table.to_vec(), Vec::new(), EvidenceClass::ByteExact),
            "argv {argv:?}",
        );
    }

    let malformed = b" id | name\n----+-----\n 1  | bad \xff\n(1 row)\n";
    assert_eq!(
        data::dispatch_streams_argv(
            &[b"psql", b"-c", b"select id, name from items"],
            malformed,
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(malformed.to_vec(), Vec::new(), EvidenceClass::ByteExact),
    );
}

#[test]
fn pup_json_fixture_matches_the_pinned_compact_shape() {
    let input = fixture("pup_skills_json.txt");
    let output =
        data::dispatch_streams_argv(&[b"pup", b"skills", b"list"], &input, b"", 0, false).unwrap();

    assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy);
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    assert!(!output.stdout.windows(2).any(|pair| pair == b"\n "));
    assert!(output.stdout.starts_with(b"[{\"description\":"));
    assert!(output.stdout.ends_with(b"\"type\":\"extension\"}]\n"));
}

#[test]
fn json_falls_open_for_multiple_roots_malformed_input_and_stderr() {
    let cases: &[StreamsCase<'_>] = &[
        (&[b"aws"], b"{\"a\":1}\n{\"b\":2}\n", b""),
        (&[b"pup"], b"{\"a\":\"oops}\n", b""),
        (
            &[b"gh", b"issue", b"list", b"--json", b"number"],
            b"{\n \"a\": 1\n}\n",
            b"warning \xff\n",
        ),
    ];

    for (argv, stdout, stderr) in cases {
        assert_eq!(
            data::dispatch_streams_argv(argv, stdout, stderr, 0, false).unwrap(),
            StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact,),
            "argv {argv:?}",
        );
    }
}

#[test]
fn ps_aux_fixture_matches_the_pinned_columnar_oracle() {
    let input = fixture("ps_aux.txt");
    let expected = concat!(
        "USER PID %CPU %MEM VSZ RSS TT STAT STARTED TIME COMMAND\n",
        "root 1 0.0 0.0 4308288 12032 ?? Ss Mon08AM 0:14.12 launchd\n",
        "~ 89 ~ ~ 4319104 6144 ~ ~ ~ 0:02.04 syslogd\n",
        "~ 91 ~ 0.1 4330880 18432 ~ ~ ~ 0:03.88 UserEventAgent\n",
        "~ 102 ~ ~ 4362752 22016 ~ ~ ~ 0:01.45 configd\n",
        "~ 103 ~ 0.0 4309888 10752 ~ ~ ~ 0:00.92 powerd\n",
        "niels 312 0.1 0.2 4512384 41984 ~ S ~ 0:44.31 Safari\n",
        "~ 318 0.0 0.1 4402176 32000 ~ ~ ~ 0:12.08 iTerm2\n",
        "~ 402 ~ 0.3 4678912 58880 ~ ~ Mon09AM 1:22.14 Electron\n",
        "~ 412 ~ 0.2 4556800 47104 ~ ~ ~ 0:36.92 Code Helper\n",
        "~ 414 ~ ~ ~ 46080 ~ ~ ~ 0:34.11 Code Helper\n",
        "~ 415 ~ ~ ~ 48128 ~ ~ ~ 0:38.44 Code Helper\n",
        "~ 501 ~ 0.1 4411648 30720 ~ ~ ~ 0:08.55 claude\n",
        "~ 502 ~ 0.0 4300928 8448 s000 Ss Mon10AM 0:00.31 -zsh\n",
        "~ 605 ~ ~ 4301952 9472 s001 Ss Tue09AM 0:00.42 -zsh\n",
        "~ 812 ~ ~ 4300928 8192 s002 S+ Wed02PM 0:00.18 tail -f log.txt\n",
    );

    assert_eq!(
        data::dispatch_streams_argv(&[b"ps", b"aux"], &input, b"warning \xff\n", 0, false).unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            b"warning \xff\n".to_vec(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn columnar_elides_fields_and_rows_for_known_text_commands() {
    let table = concat!(
        "USER   PID   COMMAND\n",
        "www    1     /usr/bin/python app.py\n",
        "www    1     /usr/bin/python app.py\n",
        "www    1     /usr/bin/python app.py\n",
        "root   2     /usr/sbin/nginx\n",
    );
    assert_eq!(
        data::dispatch_streams_argv(&[b"df", b"-h"], table.as_bytes(), b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            b"USER PID COMMAND\nwww 1 python app.py\n~ x2\nroot 2 nginx\n".to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )
    );

    let prose = b"Usage: tool [options]\n  --verbose  Show extra output\n  --help     Show help\n";
    assert_eq!(
        data::dispatch_streams_argv(&[b"brew", b"info"], prose, b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            b"Usage: tool [options]\n--verbose Show extra output\n--help Show help\n".to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn sqlite3_column_output_drops_alignment_and_separator_rows() {
    let input = fixture("sqlite3_table.txt");
    let output = data::dispatch_streams_argv(
        &[b"sqlite3", b"database.sqlite", b"select * from service"],
        &input,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(
        output,
        StreamFilterOutput::new(
            b"id\tname\tenabled\tnotes\n1\tapi\t1\tpublic HTTP service\n2\tworker\t0\tbackground worker\n3\tscheduler\t1\truns every 15 minutes\n".to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn sqlite3_preserves_empty_and_all_dash_cells() {
    let input = b"A  B  C\n---  ---  ---\n---  ---  ---\n1         3\n";
    let output = data::dispatch_streams_argv(
        &[b"sqlite3", b"database.sqlite", b"select a, b, c"],
        input,
        b"",
        0,
        false,
    )
    .unwrap();
    assert_eq!(output.stdout, b"A\tB\tC\n---\t---\t---\n1\t\t3\n");
}

#[test]
fn pup_table_fixture_matches_the_pinned_tsv_oracle() {
    let input = fixture("pup_skills_table.txt");
    let expected = expected_pup_table(&input);

    assert_eq!(
        data::dispatch_streams_argv(&[b"pup", b"skills", b"list"], &input, b"", 0, false).unwrap(),
        StreamFilterOutput::new(expected, Vec::new(), EvidenceClass::PotentiallyLossy),
    );
}

#[test]
fn machine_exact_and_failed_structured_commands_are_byte_exact() {
    let table = b"A   B   C\nx   y   z\nq   r   s\n";
    let json = b"{\n  \"a\": 1\n}\n";
    let cases: &[ExitCase<'_>] = &[
        (&[b"ps", b"-opid,comm"], table, 0),
        (&[b"psql", b"-At"], table, 0),
        (&[b"systemctl", b"show", b"nginx"], table, 0),
        (&[b"kubectl", b"get", b"pods", b"-ojson"], table, 0),
        (&[b"docker", b"ps", b"--format", b"{{.ID}}"], table, 0),
        (
            &[b"aws", b"sts", b"get-caller-identity", b"--query=x"],
            json,
            0,
        ),
        (&[b"pup", b"skills", b"list"], json, 3),
        (&[b"acli", b"jira", b"workitem", b"search"], json, 1),
        (&[b"sqlite3", b"-csv", b"database.sqlite"], table, 0),
        (&[b"sqlite3", b"-json", b"database.sqlite"], json, 0),
    ];

    for (argv, stdout, exit_code) in cases {
        assert_eq!(
            data::dispatch_streams_argv(argv, stdout, b"problem \xff\n", *exit_code, false)
                .unwrap(),
            StreamFilterOutput::new(
                stdout.to_vec(),
                b"problem \xff\n".to_vec(),
                EvidenceClass::ByteExact,
            ),
            "argv {argv:?}",
        );
    }
}

fn expected_pup_table(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() || line[0] == b'+' || is_pup_separator(line) {
            continue;
        }
        if !line.starts_with(b"|") || !line[1..].contains(&b'|') {
            output.extend_from_slice(line);
            output.push(b'\n');
            continue;
        }
        let inside = line
            .strip_prefix(b"|")
            .unwrap()
            .strip_suffix(b"|")
            .unwrap_or(&line[1..]);
        let mut fields = inside
            .split(|byte| *byte == b'|')
            .map(|field| field.trim_ascii())
            .collect::<Vec<_>>();
        while fields.last().is_some_and(|field| field.is_empty()) {
            fields.pop();
        }
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                output.push(b'\t');
            }
            output.extend_from_slice(field);
        }
        output.push(b'\n');
    }
    output
}

fn is_pup_separator(line: &[u8]) -> bool {
    line.starts_with(b"|")
        && line[1..].contains(&b'|')
        && line.iter().all(|byte| matches!(byte, b'|' | b'-'))
}

#[test]
fn cat_compacts_large_code_bodies_and_preserves_structure() {
    let mut input = Vec::new();
    input.extend_from_slice(
        b"use std::io;\n\npub struct Config {\n    value: u32,\n}\n\npub fn compute() {\n",
    );
    for index in 0..40 {
        input.extend_from_slice(format!("    let value_{index} = {index};\n").as_bytes());
    }
    input.extend_from_slice(b"}\n\npub fn second() {\n    work();\n    finish();\n}\n");
    assert!(input.len() > 512);

    let expected = concat!(
        "use std::io;\n",
        "\n",
        "pub struct Config {\n",
        "    value: u32,\n",
        "}\n",
        "\n",
        "pub fn compute() {\n",
        "    // ... (41 lines)\n",
        "}\n",
        "\n",
        "pub fn second() {\n",
        "    // ... (3 lines)\n",
        "}\n",
        "\n",
    );
    assert_eq!(
        data::dispatch_streams_argv(
            &[b"/bin/cat", b"src/lib.rs"],
            &input,
            b"warning \xff\n",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            b"warning \xff\n".to_vec(),
            EvidenceClass::PotentiallyLossy,
        )
    );
}

#[test]
fn cat_data_small_unknown_and_exact_invocations_are_byte_exact() {
    let code = vec![b'x'; 600];
    let json = [b"{\"a\":1}\n".as_slice(), &vec![b' '; 600]].concat();
    for (argv, stdout) in [
        (&[b"cat".as_slice(), b"data.json"][..], json.as_slice()),
        (&[b"cat".as_slice(), b"notes.unknown"][..], code.as_slice()),
        (
            &[b"cat".as_slice(), b"-n", b"src/lib.rs"][..],
            code.as_slice(),
        ),
        (
            &[b"cat".as_slice(), b"src/lib.rs"][..],
            b"pub fn x() {}\n".as_slice(),
        ),
    ] {
        assert_eq!(
            data::dispatch_streams_argv(argv, stdout, b"problem \xff\n", 0, false).unwrap(),
            StreamFilterOutput::new(
                stdout.to_vec(),
                b"problem \xff\n".to_vec(),
                EvidenceClass::ByteExact,
            ),
            "argv {argv:?}",
        );
    }
}

#[test]
fn bat_and_batcat_share_only_the_plain_cat_compaction_contract() {
    let input = fixture("bat_code.rs");
    assert!(input.len() > 512);

    for argv in [
        &[b"bat".as_slice(), b"src/lib.rs"][..],
        &[b"batcat".as_slice(), b"src/lib.rs"][..],
        &[b"bat".as_slice()][..],
    ] {
        let output = data::dispatch_streams_argv(argv, &input, b"", 0, false).unwrap();
        assert_eq!(output.evidence, EvidenceClass::PotentiallyLossy, "{argv:?}");
        if argv.len() == 2 {
            assert!(
                output
                    .stdout
                    .windows(b"// ... (".len())
                    .any(|part| part == b"// ... (")
            );
        }
    }

    for argv in [
        &[b"bat".as_slice(), b"--plain", b"src/lib.rs"][..],
        &[b"bat".as_slice(), b"-n", b"src/lib.rs"][..],
        &[b"batcat".as_slice(), b"--style=plain", b"src/lib.rs"][..],
        &[b"bat".as_slice(), b"one.rs", b"two.rs"][..],
    ] {
        let output = data::dispatch_streams_argv(argv, &input, b"", 0, false).unwrap();
        assert_eq!(output.evidence, EvidenceClass::ByteExact, "{argv:?}");
    }
}

#[test]
fn lossless_and_invalid_argv_are_preserved_or_rejected() {
    let stdout = b"A   B   C\n\xff   y   z\nq   r   s\n";
    assert_eq!(
        data::dispatch_streams_argv(&[b"ps", b"aux"], stdout, b"err \xfe\n", 0, true).unwrap(),
        StreamFilterOutput::new(
            stdout.to_vec(),
            b"err \xfe\n".to_vec(),
            EvidenceClass::ByteExact,
        )
    );
    assert_eq!(
        data::dispatch_streams_argv(&[], b"", b"", 0, false),
        Err(tapas::filters::FilterError::InvalidInput),
    );
}

#[test]
fn sigil_rle_is_collision_safe_and_byte_reversible() {
    let input = b"abcdefghijklmnopX\nabcdefghijklmnopY\n\x01natural\nabcdefghijklmnop\x01Z\n";
    let expected = b"abcdefghijklmnopX\n\x01Y\n\x01\x01natural\nabcdefghijklmnop\x01Z\n";

    let encoded = data::sigil_rle::encode(input);
    assert_eq!(encoded, expected);
    assert_eq!(data::sigil_rle::decode(&encoded), input);
}

#[test]
fn whitespace_rle_round_trips_long_runs_literals_and_invalid_bytes() {
    let mut input = b"head".to_vec();
    input.extend(std::iter::repeat_n(b' ', 260));
    input.extend_from_slice(b"tail\x01end");
    let encoded = data::ws_rle::encode(&input);

    assert_eq!(&encoded[..6], &[b'h', b'e', b'a', b'd', 0x01, 255]);
    assert_eq!(&encoded[6..11], b"     ");
    assert!(encoded.ends_with(b"tail\x01\x00end"));
    assert_eq!(data::ws_rle::decode(&encoded).unwrap(), input);
    assert_eq!(
        data::ws_rle::decode(b"bad\x01\x02"),
        Err(tapas::filters::FilterError::InvalidInput),
    );
    assert_eq!(
        data::ws_rle::decode(b"bad\x01"),
        Err(tapas::filters::FilterError::InvalidInput),
    );
}
