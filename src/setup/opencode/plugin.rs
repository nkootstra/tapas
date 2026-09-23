use std::path::Path;

use super::super::json::{self, Value};

pub(super) fn paths_are_utf8(config_path: &Path, executable: &Path) -> bool {
    config_path.to_str().is_some() && executable.to_str().is_some()
}

pub(super) fn generate(executable: &Path) -> Vec<u8> {
    let mut quoted = Vec::new();
    json::write_string(executable.as_os_str().as_encoded_bytes(), &mut quoted);
    let mut output =
        b"// Managed by Tapas. Do not edit; use `tapas --unsetup opencode`.\nconst tapas = "
            .to_vec();
    output.extend_from_slice(&quoted);
    output.extend_from_slice(
        br#";
let warned = false;

function rewrite(command, workdir) {
  try {
    const result = Bun.spawnSync([tapas, "--hook-eval", "opencode"], {
      stdin: new TextEncoder().encode(
        JSON.stringify({ cwd: workdir, tool_input: { command } }),
      ),
      stdout: "pipe",
      stderr: "ignore",
      timeout: 1000,
      maxBuffer: 65536,
    });
    const text = result.exitCode === 0 ? result.stdout.toString() : "";
    const updated = text.endsWith("\n") ? text.slice(0, -1) : "";
    return updated || command;
  } catch (error) {
    if (!warned) {
      warned = true;
      console.warn("Tapas OpenCode hook failed; command left unchanged");
    }
    return command;
  }
}

// OpenCode V1 entrypoint: V1 reads `server()` from the default export.
async function server() {
  return {
    "tool.execute.before": async (input, output) => {
      if (input.tool !== "bash" || typeof output.args?.command !== "string") return;
      output.args.command = rewrite(output.args.command, output.args.workdir);
    },
  };
}

// OpenCode V2 entrypoint: a stable id and setup(ctx). V1 ignores `setup`.
// V2 names the shell tool `shell` (and also accepts `bash`).
export default {
  id: "tapas",
  server,
  async setup(ctx) {
    await ctx.tool.hook("execute.before", (event) => {
      if ((event.tool !== "shell" && event.tool !== "bash") || typeof event.input?.command !== "string") return;
      event.input.command = rewrite(event.input.command, event.input.workdir);
    });
  },
};
"#,
    );
    output
}

pub(super) fn ownership(path: &Path, executable: &Path, content: &[u8]) -> Value {
    Value::Object(vec![
        (b"kind".to_vec(), Value::String(b"opencode-plugin".to_vec())),
        (
            b"path".to_vec(),
            Value::String(path.as_os_str().as_encoded_bytes().to_vec()),
        ),
        (
            b"executable".to_vec(),
            Value::String(executable.as_os_str().as_encoded_bytes().to_vec()),
        ),
        (b"content".to_vec(), Value::String(content.to_vec())),
    ])
}

pub(super) fn owned_matches(owned: &Value, path: &Path, content: &[u8]) -> bool {
    matches!(owned.get(b"kind"), Some(Value::String(kind)) if kind == b"opencode-plugin")
        && matches!(owned.get(b"path"), Some(Value::String(value)) if value == path.as_os_str().as_encoded_bytes())
        && matches!(owned.get(b"content"), Some(Value::String(value)) if value == content)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use super::{generate, paths_are_utf8};

    #[test]
    fn paths_must_be_utf8() {
        let invalid = Path::new(OsStr::from_bytes(b"/tmp/tapas-\xff"));

        assert!(!paths_are_utf8(Path::new("/tmp/tapas.js"), invalid));
        assert!(!paths_are_utf8(invalid, Path::new("/tmp/tapas")));
        assert!(paths_are_utf8(
            Path::new("/tmp/tapas.js"),
            Path::new("/tmp/tapas")
        ));
    }

    #[test]
    fn generated_plugin_bytes_are_frozen() {
        let expected = concat!(
            "// Managed by Tapas. Do not edit; use `tapas --unsetup opencode`.\n",
            "const tapas = \"/tmp/tapas\";\n",
            "let warned = false;\n",
            "\n",
            "function rewrite(command, workdir) {\n",
            "  try {\n",
            "    const result = Bun.spawnSync([tapas, \"--hook-eval\", \"opencode\"], {\n",
            "      stdin: new TextEncoder().encode(\n",
            "        JSON.stringify({ cwd: workdir, tool_input: { command } }),\n",
            "      ),\n",
            "      stdout: \"pipe\",\n",
            "      stderr: \"ignore\",\n",
            "      timeout: 1000,\n",
            "      maxBuffer: 65536,\n",
            "    });\n",
            "    const text = result.exitCode === 0 ? result.stdout.toString() : \"\";\n",
            "    const updated = text.endsWith(\"\\n\") ? text.slice(0, -1) : \"\";\n",
            "    return updated || command;\n",
            "  } catch (error) {\n",
            "    if (!warned) {\n",
            "      warned = true;\n",
            "      console.warn(\"Tapas OpenCode hook failed; command left unchanged\");\n",
            "    }\n",
            "    return command;\n",
            "  }\n",
            "}\n",
            "\n",
            "// OpenCode V1 entrypoint: V1 reads `server()` from the default export.\n",
            "async function server() {\n",
            "  return {\n",
            "    \"tool.execute.before\": async (input, output) => {\n",
            "      if (input.tool !== \"bash\" || typeof output.args?.command !== \"string\") return;\n",
            "      output.args.command = rewrite(output.args.command, output.args.workdir);\n",
            "    },\n",
            "  };\n",
            "}\n",
            "\n",
            "// OpenCode V2 entrypoint: a stable id and setup(ctx). V1 ignores `setup`.\n",
            "// V2 names the shell tool `shell` (and also accepts `bash`).\n",
            "export default {\n",
            "  id: \"tapas\",\n",
            "  server,\n",
            "  async setup(ctx) {\n",
            "    await ctx.tool.hook(\"execute.before\", (event) => {\n",
            "      if ((event.tool !== \"shell\" && event.tool !== \"bash\") || typeof event.input?.command !== \"string\") return;\n",
            "      event.input.command = rewrite(event.input.command, event.input.workdir);\n",
            "    });\n",
            "  },\n",
            "};\n",
        );

        assert_eq!(generate(Path::new("/tmp/tapas")), expected.as_bytes());
    }
}
