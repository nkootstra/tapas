use tapas::filters::{EvidenceClass, StreamFilterOutput, build};

const FIXTURES: &str = "compat/smll-v1.9.0/fixtures";
type BuildCase<'a> = (&'a [&'a [u8]], Vec<u8>, i32, &'a [u8]);

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/{FIXTURES}/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn cargo_build_fixture_matches_the_pinned_wrapper_oracle() {
    let input = fixture("tests/fixtures/cargo_build.txt");
    let expected = concat!(
        "warning: unused variable: `tmp`\n",
        " --> src/lib.rs:42:9\n",
        "   |\n",
        "42 |     let tmp = 5;\n",
        "   |         ^^^ help: if this is intentional, prefix it with an underscore: `_tmp`\n",
        "   |\n",
        "   = note: `#[warn(unused_variables)]` on by default\n",
        "\n",
        "cargo: Finished dev; 7 crates\n",
    );

    assert_eq!(
        build::dispatch_streams_argv(&[b"cargo", b"build"], &input, b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn make_ninja_go_and_zig_builds_match_the_pinned_wrapper_oracles() {
    let cases: &[BuildCase<'_>] = &[
        (
            &[b"make"],
            fixture("tests/fixtures/make_build.txt"),
            0,
            concat!(
                "src/baz.c:18:12: warning: unused variable 'tmp' [-Wunused-variable]\n",
                "   18 |     int tmp = 0;\n",
                "      |         ^~~\n",
                "Compiled 5 (make)\n",
            )
            .as_bytes(),
        ),
        (
            &[b"ninja"],
            fixture("tests/fixtures/ninja_build.txt"),
            0,
            concat!(
                "main.c:1:22: warning: unused variable 'unused' [-Wunused-variable]\n",
                "    1 | int main(void) { int unused = 1; return 0; }\n",
                "      |                      ^~~~~~\n",
                "1 warning generated.\n",
                "built 2 (ninja)\n",
            )
            .as_bytes(),
        ),
        (
            &[b"go", b"build", b"./..."],
            fixture("tests/fixtures/go_build.txt"),
            1,
            concat!(
                "internal/auth/token.go:42:9: declared and not used: claims\n",
                "Compiled 6 (go)\n",
            )
            .as_bytes(),
        ),
        (
            &[b"zig", b"build"],
            b"info: compiling app\nerror: expected type bool\n".to_vec(),
            1,
            b"info: compiling app\nerror: expected type bool\n",
        ),
    ];

    for (argv, input, exit_code, expected) in cases {
        assert_eq!(
            build::dispatch_streams_argv(argv, input, b"", *exit_code, false).unwrap(),
            StreamFilterOutput::new(
                expected.to_vec(),
                Vec::new(),
                if *exit_code == 0 {
                    EvidenceClass::PotentiallyLossy
                } else {
                    EvidenceClass::FactComplete
                },
            ),
            "argv {argv:?}",
        );
    }
}

#[test]
fn npm_build_fixture_matches_the_pinned_bundler_oracle() {
    let input = fixture("tests/fixtures/vite_build.txt");
    let expected = concat!(
        "vite v5.4.2 building for production...\n",
        "✓ 1248 modules transformed.\n",
        "\n",
        "(!) Some chunks are larger than 500 kB after minification. Consider:\n",
        "- Using dynamic import() to code-split the application\n",
        "- Use build.rollupOptions.output.manualChunks to improve chunking\n",
        "- Adjust chunk size limit for this warning via build.chunkSizeWarningLimit\n",
        "\n",
        "✓ built in 12.34s\n",
        "\n",
        "assets x10; largest:\n",
        "- dist/assets/index-Bj4i90KS.js 485.20 kB\n",
        "- dist/assets/vendor-chart-Lk2P0nMz.js 321.10 kB\n",
        "- dist/assets/vendor-react-Qf8X1pPw.js 142.65 kB\n",
        "- dist/assets/index-Bj4i90KS.css 18.42 kB\n",
        "- dist/assets/dashboard-Mf3K2sxR.js 12.18 kB\n",
    );

    assert_eq!(
        build::dispatch_streams_argv(&[b"npm", b"run", b"build"], &input, b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn next_webpack_and_turbo_outputs_match_the_pinned_oracles() {
    let next = fixture("tests/fixtures/next_build.txt");
    let next_expected = concat!(
        "  ▲ Next.js 14.2.5\n",
        "  - Environments: .env.local, .env\n",
        "\n",
        " ✓ Compiled successfully\n",
        "   Linting and checking validity of types ...\n",
        "\n",
        "./app/dashboard/page.tsx\n",
        "12:8  Warning: 'useEffect' is defined but never used.  @typescript-eslint/no-unused-vars\n",
        "\n",
        "./app/settings/page.tsx\n",
        "27:14  Warning: React Hook useEffect has a missing dependency: 'userId'.  react-hooks/exhaustive-deps\n",
        "\n",
        " ✓ Collecting page data\n",
        " ✓ Generating static pages (8/8)\n",
        " ✓ Collecting build traces\n",
        " ✓ Finalizing page optimization\n",
        "\n",
        "Route (app)                              Size     First Load JS\n",
        "┌ ○ /                                    1.84 kB         92.4 kB\n",
        "├ ○ /_not-found                          885 B          85.4 kB\n",
        "├ ○ /about                               142 B          84.7 kB\n",
        "├ ○ /dashboard                           4.21 kB         118 kB\n",
        "├ ○ /login                               2.34 kB         97.1 kB\n",
        "├ ○ /settings                            3.18 kB         115 kB\n",
        "├ λ /api/auth/[...nextauth]              0 B                0 B\n",
        "└ λ /api/foo                             0 B                0 B\n",
        "+ First Load JS shared by all            84.6 kB\n",
        "  ├ chunks/472-9af3d2.js                 29 kB\n",
        "  ├ chunks/fd9d1056-bc8e2a.js            53.5 kB\n",
        "  └ other shared chunks (total)          1.92 kB\n",
        "\n",
        "○  (Static)   prerendered as static content\n",
        "λ  (Dynamic)  server-rendered on demand\n",
        "\n",
    );
    assert_eq!(
        build::dispatch_streams_argv(&[b"next", b"build"], &next, b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            next_expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let webpack = fixture("tests/fixtures/webpack_build.txt");
    let webpack_expected = concat!(
        "webpack 5.107.2 compiled successfully in 124 ms\n",
        "\n",
        "assets x1; largest:\n",
        "- asset bundle.js 49 bytes [emitted] [minimized] (name: main)\n",
    );
    assert_eq!(
        build::dispatch_streams_argv(&[b"webpack"], &webpack, b"", 0, false).unwrap(),
        StreamFilterOutput::new(
            webpack_expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let turbo = fixture("benchmarks/smll-vs-rtk/fixtures/turbo_error.txt");
    assert_eq!(
        build::dispatch_streams_argv(&[b"turbo", b"build"], &turbo, b"", 1, false).unwrap(),
        StreamFilterOutput::new(
            b"> myapp:lint\n\nError: src/index.ts(5,1): error TS2304\n\n".to_vec(),
            Vec::new(),
            EvidenceClass::FactComplete,
        ),
    );
}

#[test]
fn dotnet_build_test_format_and_restore_match_the_pinned_oracles() {
    let cases: &[BuildCase<'_>] = &[
        (
            &[b"dotnet", b"build"],
            fixture("benchmarks/smll-vs-rtk/fixtures/dotnet_build_failed.txt"),
            1,
            concat!(
                "src/Program.cs(10,5): error CS1002: ; expected [/home/user/MyApp/MyApp.csproj]\n",
                "Build FAILED.\n",
                "    0 Warning(s)\n",
                "    1 Error(s)\n",
            )
            .as_bytes(),
        ),
        (
            &[b"dotnet", b"test"],
            fixture("benchmarks/smll-vs-rtk/fixtures/dotnet_test_failed.txt"),
            1,
            concat!(
                "[xUnit.net 00:00:00.11]     MyApp.Tests.CalculatorTests.Subtract [FAIL]\n",
                "  Failed MyApp.Tests.CalculatorTests.Subtract [4 ms]\n",
                "  Error Message:\n",
                "   Assert.Equal() Failure: Values differ\n",
                "Expected: 2\n",
                "Actual:   3\n",
                "  Stack Trace:\n",
                "     at MyApp.Tests.CalculatorTests.Subtract() in /home/user/MyApp/tests/CalculatorTests.cs:line 8\n",
                "Failed!  - Failed:     1, Passed:     0, Skipped:     0, Total:     1, Duration: 13 ms - MyApp.dll (net8.0)\n",
            )
            .as_bytes(),
        ),
        (
            &[b"dotnet", b"format"],
            b"Formatting code files in workspace\nWarnings were encountered while loading the workspace.\n".to_vec(),
            0,
            b"",
        ),
        (
            &[b"dotnet", b"restore"],
            b"Determining projects to restore...\nRestored app.csproj (in 1.2 sec).\n".to_vec(),
            0,
            b"Restored app.csproj (in 1.2 sec).\n",
        ),
    ];

    for (argv, input, exit_code, expected) in cases {
        assert_eq!(
            build::dispatch_streams_argv(argv, input, b"", *exit_code, false).unwrap(),
            StreamFilterOutput::new(
                expected.to_vec(),
                Vec::new(),
                if *exit_code == 0 {
                    EvidenceClass::PotentiallyLossy
                } else {
                    EvidenceClass::FactComplete
                },
            ),
            "argv {argv:?}",
        );
    }
}

#[test]
fn gradle_and_maven_wrappers_match_the_pinned_failure_oracles() {
    let gradle_cases: &[(&[&[u8]], &str, &str)] = &[
        (
            &[b"gradle", b"build"],
            "benchmarks/smll-vs-rtk/fixtures/gradle_build_failed.txt",
            concat!(
                "> Task :app:compileDebugKotlin FAILED\n",
                "FAILURE: Build failed with an exception.\n",
                "* What went wrong:\n",
                "Execution failed for task ':app:compileDebugKotlin'.\n",
                "> Compilation error. See log for more details\n",
                "e: /home/user/MyApp/app/src/main/java/com/example/myapp/MainActivity.kt: (42, 5): Unresolved reference: MyService\n",
                "e: /home/user/MyApp/app/src/main/java/com/example/myapp/MainActivity.kt: (56, 17): Type mismatch: inferred type is String but Int was expected\n",
                "BUILD FAILED in 12s\n",
            ),
        ),
        (
            &[b"gradlew", b"test"],
            "benchmarks/smll-vs-rtk/fixtures/gradle_test_failed.txt",
            concat!(
                "com.example.myapp.CalculatorTest > testSubtraction FAILED\n",
                "java.lang.AssertionError: expected:<3> but was:<-1>\n",
                "at org.junit.Assert.fail(Assert.java:89)\n",
                "at org.junit.Assert.assertEquals(Assert.java:197)\n",
                "at com.example.myapp.CalculatorTest.testSubtraction(CalculatorTest.kt:25)\n",
                "com.example.myapp.MainViewModelTest > loadDataError FAILED\n",
                "kotlin.NotImplementedError: An operation is not implemented: TODO\n",
                "at com.example.myapp.MainViewModelTest.loadDataError(MainViewModelTest.kt:45)\n",
                "5 tests completed, 2 failed\n",
                "There were failing tests. See the report at: file:///home/user/MyApp/app/build/reports/tests/testDebugUnitTest/index.html\n",
                "BUILD FAILED in 22s\n",
            ),
        ),
    ];
    for (argv, fixture_path, expected) in gradle_cases {
        let input = fixture(fixture_path);
        assert_eq!(
            build::dispatch_streams_argv(argv, &input, b"", 1, false).unwrap(),
            StreamFilterOutput::new(
                expected.as_bytes().to_vec(),
                Vec::new(),
                EvidenceClass::FactComplete,
            ),
            "argv {argv:?}",
        );
    }

    let maven = fixture("benchmarks/smll-vs-rtk/fixtures/mvn_build_failed.txt");
    let maven_expected = concat!(
        "[ERROR] /src/main/java/Main.java:[10,5] cannot find symbol\n",
        "symbol: method foo()\n",
        "BUILD FAILURE\n",
        "[INFO] Total time: 2.543 s\n",
    );
    for command in [b"mvn".as_slice(), b"mvnw"] {
        assert_eq!(
            build::dispatch_streams_argv(&[command, b"package"], &maven, b"", 1, false).unwrap(),
            StreamFilterOutput::new(
                maven_expected.as_bytes().to_vec(),
                Vec::new(),
                EvidenceClass::FactComplete,
            ),
            "command {command:?}",
        );
    }
}

#[test]
fn apple_build_and_uv_package_fallbacks_match_the_pinned_smokes() {
    assert_eq!(
        build::dispatch_streams_argv(
            &[b"swift", b"build"],
            b"Compile Swift Module App\nBuild complete!\n",
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"ok\n".to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let xcode = concat!(
        "CompileC /tmp/App.o /tmp/App.m\n",
        "SwiftCompile normal arm64 /tmp/App.swift\n",
        "/tmp/App.swift:8:4: warning: immutable value was never used\n",
        "** BUILD SUCCEEDED **\n",
    );
    assert_eq!(
        build::dispatch_streams_argv(&[b"xcodebuild", b"build"], xcode.as_bytes(), b"", 0, false,)
            .unwrap(),
        StreamFilterOutput::new(
            concat!(
                "SwiftCompile normal arm64 /tmp/App.swift\n",
                "/tmp/App.swift:8:4: warning: immutable value was never used\n",
                "** BUILD SUCCEEDED **\n",
            )
            .as_bytes()
            .to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let uv = fixture("benchmarks/smll-vs-rtk/fixtures/uv_pip_install.txt");
    let uv_expected = concat!(
        "Installed 5 packages in 23ms\n",
        " + certifi==2023.11.17\n",
        " + charset-normalizer==3.3.2\n",
        " + idna==3.6\n",
        " + requests==2.31.0\n",
        " + urllib3==2.1.0\n",
    );
    assert_eq!(
        build::dispatch_streams_argv(
            &[b"uv", b"pip", b"install", b"-r", b"requirements.txt"],
            &uv,
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            uv_expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    assert_eq!(
        build::dispatch_streams_argv(
            &[b"uvx", b"ruff", b"check"],
            b"Installed 1 package in 9ms\nAll checks passed!\n",
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"ok\n".to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn exact_modes_and_content_gates_preserve_both_streams() {
    let stdout = b"Build succeeded.\n";
    let stderr = b"query diagnostic\n";
    for argv in [
        &[b"dotnet".as_slice(), b"build", b"-getProperty:Version"][..],
        &[b"dotnet".as_slice(), b"build", b"/GETITEM:Compile"][..],
        &[b"dotnet".as_slice(), b"build", b"--getTargetResult:Build"][..],
        &[b"cargo".as_slice(), b"build", b"--help"][..],
    ] {
        assert_eq!(
            build::dispatch_streams_argv(argv, stdout, stderr, 0, false).unwrap(),
            StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact,),
            "argv {argv:?}",
        );
    }

    assert_eq!(
        build::dispatch_streams_argv(
            &[b"npm", b"run", b"build"],
            b"\xffvite v5.4.2 building for production...\n",
            b"\xfeopaque diagnostic\n",
            0,
            true,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"\xffvite v5.4.2 building for production...\n".to_vec(),
            b"\xfeopaque diagnostic\n".to_vec(),
            EvidenceClass::ByteExact,
        ),
    );

    let vite = fixture("tests/fixtures/vite_build.txt");
    for argv in [
        &[b"npm".as_slice(), b"run", b"lint"][..],
        &[b"npm".as_slice(), b"run", b"build"][..],
    ] {
        let input = if argv[2] == b"lint" {
            vite.as_slice()
        } else {
            b"application output without a build signature\n"
        };
        assert_eq!(
            build::dispatch_streams_argv(argv, input, b"", 0, false).unwrap(),
            StreamFilterOutput::new(input.to_vec(), Vec::new(), EvidenceClass::ByteExact),
            "argv {argv:?}",
        );
    }

    assert_eq!(
        build::dispatch_streams_argv(
            &[b"dotnet", b"build", b"--", b"-getProperty:Version"],
            b"noise\nBuild succeeded.\n",
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"Build succeeded.\n".to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}

#[test]
fn command_paths_and_failure_boundaries_are_preserved() {
    assert_eq!(
        build::dispatch_streams_argv(
            &[b"/usr/local/bin/cargo", b"build"],
            b"   Compiling demo v0.1.0\n    Finished dev [unoptimized] target(s) in 0.1s\n",
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"cargo: Finished dev; 1 crates\n".to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let stdout = b"   Compiling demo v0.1.0\n";
    let stderr = b"linker terminated unexpectedly\n";
    assert_eq!(
        build::dispatch_streams_argv(&[b"cargo", b"build"], stdout, stderr, 1, false).unwrap(),
        StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact,),
    );

    assert_eq!(
        build::dispatch_streams_argv(
            &[b"npm", b"run", b"build"],
            b"vite v5.4.2 building for production...\ntransforming...\n",
            b"Error: plugin exploded\n",
            1,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            b"vite v5.4.2 building for production...\ntransforming...\n".to_vec(),
            b"Error: plugin exploded\n".to_vec(),
            EvidenceClass::ByteExact,
        ),
    );
}

#[test]
fn build_truncation_notice_names_tapas_raw_mode() {
    let mut input = b"webpack 5.0 compiled with 201 errors\n".to_vec();
    for index in 0..201 {
        input.extend_from_slice(format!("ERROR in src/file-{index}.js\n").as_bytes());
    }

    let output = build::dispatch_streams_argv(&[b"webpack"], &input, b"", 1, false).unwrap();

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
fn large_build_fixtures_match_the_pinned_wrapper_oracles() {
    let cargo_expected = concat!(
        "warning: unused import: `std::collections::HashMap`\n",
        " --> src/lib.rs:3:5\n",
        "  |\n",
        "3 | use std::collections::HashMap;\n",
        "  |     ^^^^^^^^^^^^^^^^^^^^^^^^^\n",
        "warning: variable does not need to be mutable\n",
        " --> src/core.rs:77:9\n",
        "  |\n",
        "77 |     let mut tally = 0;\n",
        "  |         ----^^^^^\n",
        "error[E0308]: mismatched types\n",
        " --> src/main.rs:17:5\n",
        "  |\n",
        "17 |     return x;\n",
        "  |     ^^^^^^^^ expected `()`, found integer\n",
        "cargo: Finished dev; 500 crates\n",
    );
    assert_eq!(
        build::dispatch_streams_argv(
            &[b"cargo", b"build"],
            &fixture("tests/fixtures/large/cargo_build.txt"),
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            cargo_expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );

    let go_expected = concat!(
        "internal/auth/token.go:42:9: error: declared and not used: claims\n",
        "internal/auth/token.go:51:2: error: cannot use invalid (type untyped int) as type string in argument to fmt.Println\n",
        "Compiled 500 (go)\n",
    );
    assert_eq!(
        build::dispatch_streams_argv(
            &[b"go", b"build", b"./..."],
            &fixture("tests/fixtures/large/go_build.txt"),
            b"",
            1,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            go_expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::FactComplete,
        ),
    );

    let make_expected = concat!(
        "src/mod_150.c:32:12: warning: unused variable 'tmp' [-Wunused-variable]\n",
        "   32 |     int tmp = 0;\n",
        "      |         ^~~\n",
        "src/mod_350.c:18:9: warning: implicit declaration of function 'do_thing' [-Wimplicit-function-declaration]\n",
        "   18 |     do_thing();\n",
        "      |     ^~~~~~~~\n",
        "Compiled 501 (make)\n",
    );
    assert_eq!(
        build::dispatch_streams_argv(
            &[b"make"],
            &fixture("tests/fixtures/large/make_build.txt"),
            b"",
            0,
            false,
        )
        .unwrap(),
        StreamFilterOutput::new(
            make_expected.as_bytes().to_vec(),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ),
    );
}
