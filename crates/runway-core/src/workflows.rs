/// Warp-compatible workflow files: parameterized commands with
/// `{{placeholder}}` syntax, loaded from `~/.config/runway/workflows/*.yaml`.
/// Format-compatible with Warp's public workflow spec so its community
/// library drops in unchanged.
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkflowArg {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<WorkflowArg>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Workflow {
    pub fn parse(yaml: &str) -> Result<Workflow, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(yaml)
    }

    /// Command with defaults substituted where available; placeholder spans
    /// (byte ranges into the returned string) for the rest, in order.
    pub fn instantiate(&self) -> (String, Vec<(usize, usize)>) {
        let mut out = String::new();
        let mut spans = Vec::new();
        let src = &self.command;
        let mut rest = src.as_str();
        while let Some(open) = rest.find("{{") {
            out.push_str(&rest[..open]);
            let after = &rest[open + 2..];
            let Some(close) = after.find("}}") else {
                out.push_str(&rest[open..]);
                rest = "";
                break;
            };
            let raw_name = after[..close].trim();
            let default = self
                .arguments
                .iter()
                .find(|a| a.name == raw_name)
                .and_then(|a| a.default_value.clone());
            let start = out.len();
            match default {
                Some(v) if !v.is_empty() => out.push_str(&v),
                _ => out.push_str(raw_name),
            }
            spans.push((start, out.len()));
            rest = &after[close + 2..];
        }
        out.push_str(rest);
        (out, spans)
    }
}

/// Sample workflows embedded so the picker is never empty.
pub const SAMPLES: &[&str] = &[
    r#"name: Interactive rebase last N commits
command: git rebase -i HEAD~{{count}}
description: Rewrite the last N commits interactively
arguments:
  - name: count
    description: Number of commits
    default_value: "3"
tags: [git]
"#,
    r#"name: Serve current directory over HTTP
command: python3 -m http.server {{port}}
description: Quick static file server
arguments:
  - name: port
    default_value: "8000"
tags: [web]
"#,
    r#"name: Find large files
command: find {{path}} -type f -size +{{size}} -exec ls -lh {} +
description: List files bigger than a threshold
arguments:
  - name: path
    default_value: "."
  - name: size
    description: e.g. 100M
    default_value: "100M"
tags: [files]
"#,
];

/// Embedded samples + user files from `~/.config/runway/workflows/`.
pub fn load_all() -> Vec<Workflow> {
    let mut out: Vec<Workflow> = SAMPLES
        .iter()
        .filter_map(|s| Workflow::parse(s).ok())
        .collect();
    if let Some(dir) = crate::config::config_dir().map(|d| d.join("workflows"))
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for e in entries.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str());
            if matches!(ext, Some("yaml") | Some("yml"))
                && let Ok(body) = std::fs::read_to_string(&p)
            {
                match Workflow::parse(&body) {
                    Ok(w) => out.push(w),
                    Err(err) => log::warn!("workflow {p:?}: {err}"),
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_warp_format_with_block_scalar() {
        let w = Workflow::parse(
            "name: multi\ndescription: block scalar\ncommand: |\n  echo one\n  echo {{two}}\narguments:\n  - name: two\n    default_value: \"2\"\n",
        )
        .unwrap();
        assert!(w.command.contains("echo one"));
        let (cmd, spans) = w.instantiate();
        assert!(cmd.contains("echo 2"));
        assert_eq!(spans.len(), 1);
        assert_eq!(&cmd[spans[0].0..spans[0].1], "2");
    }

    #[test]
    fn placeholder_without_default_keeps_name() {
        let w = Workflow::parse("name: x\ncommand: ssh {{host}} -p {{port}}\n").unwrap();
        let (cmd, spans) = w.instantiate();
        assert_eq!(cmd, "ssh host -p port");
        assert_eq!(spans.len(), 2);
        assert_eq!(&cmd[spans[0].0..spans[0].1], "host");
        assert_eq!(&cmd[spans[1].0..spans[1].1], "port");
    }

    #[test]
    fn samples_all_parse() {
        assert_eq!(load_samples_count(), SAMPLES.len());
    }

    fn load_samples_count() -> usize {
        SAMPLES
            .iter()
            .filter_map(|s| Workflow::parse(s).ok())
            .count()
    }
}
