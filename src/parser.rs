//! Tiny DSL for context-aware key rules.
//!
//! A payload is a sequence of clauses separated by newlines or `;`. Each
//! clause is one of:
//!
//!   default: <action>
//!   when <proc1>[,<proc2>...]: <action>
//!
//! Where `<action>` is one of:
//!
//!   key <keyspec>                 # friendly key notation (e.g. ctrl+j)
//!   $source                       # forward the binding's own key
//!   write <text-with-escapes>     # raw bytes to the focused pane
//!   do <zellij-action> [<arg>...] # built-in Zellij command
//!   run <command> [<arg>...]      # exec an external command headlessly
//!   noop                          # swallow the key
//!
//! Comments start with `#` and run to end-of-line.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Action {
    /// Send raw bytes to the focused pane.
    Write(Vec<u8>),
    /// Forward the binding's *own* key — i.e. send whatever the key named in
    /// the dispatching `MessagePlugin`'s `name` field would normally produce.
    /// Resolved at execution time (in `main.rs`) because the `name` lives on
    /// the `PipeMessage`, not in the rule text. Written `$source` (or
    /// `source`) in the DSL. `kitty` forces the kitty keyboard-protocol
    /// (CSI-u) encoding even when `name` lacks a `kitty:` prefix — written
    /// `kitty:$source`.
    Source { kitty: bool },
    /// Invoke a built-in Zellij command (e.g. `move_focus down`).
    Do { name: String, args: Vec<String> },
    /// Re-dispatch the keystroke into another plugin via Zellij's
    /// plugin-to-plugin messaging. Mirrors a Zellij KDL `MessagePlugin`
    /// invocation: `url` identifies the target plugin, `name`/`payload`
    /// become the `PipeMessage` fields, and `config` flows into the target
    /// plugin's `load()` configuration map.
    Pipe {
        url: String,
        name: Option<String>,
        payload: Option<String>,
        config: BTreeMap<String, String>,
    },
    /// Run an external command headlessly via the host (no pane is spawned,
    /// so there's no visual flash). `argv[0]` is the program; the rest are
    /// arguments. Two placeholder tokens are substituted at execution time
    /// against the focused terminal pane: `{pid}` → that pane's root process
    /// PID, and `{pane_id}` → its numeric terminal pane id. This lets a
    /// helper script resolve e.g. the pane's cwd (via `lsof -p {pid}`)
    /// without depending on any ZELLIJ_* environment variable, which the
    /// host does not guarantee for run_command children.
    Run { argv: Vec<String> },
    /// Swallow the key.
    Noop,
}

#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    pub default: Option<Action>,
    pub overrides: Vec<Override>,
}

#[derive(Debug, Clone)]
pub struct Override {
    pub processes: Vec<String>,
    pub action: Action,
}

impl RuleSet {
    /// Pick the action whose `when` clause matches any candidate (case-
    /// insensitive). Falls back to the default action, then to a no-op.
    ///
    /// Candidates are typically the focused pane's immediate foreground
    /// command plus the basenames of every descendant process — that way an
    /// `fzf` rule fires even when fzf was spawned indirectly by zsh tab
    /// completion or a shell function.
    pub fn choose(&self, candidates: &[&str]) -> Action {
        if !candidates.is_empty() {
            for ov in &self.overrides {
                if ov
                    .processes
                    .iter()
                    .any(|p| candidates.iter().any(|c| p.eq_ignore_ascii_case(c)))
                {
                    return ov.action.clone();
                }
            }
        }
        self.default.clone().unwrap_or(Action::Noop)
    }
}

pub fn parse(input: &str) -> Result<RuleSet, String> {
    let mut set = RuleSet::default();

    for raw in input.split(['\n', ';']) {
        let clause = strip_comment(raw).trim();
        if clause.is_empty() {
            continue;
        }

        if let Some(rest) = clause.strip_prefix("default:") {
            set.default = Some(parse_action(rest.trim())?);
            continue;
        }

        let body = clause
            .strip_prefix("when ")
            .or_else(|| clause.strip_prefix("when\t"));
        if let Some(body) = body {
            let colon = body
                .find(':')
                .ok_or_else(|| format!("missing ':' in 'when' clause: {clause}"))?;
            let processes: Vec<String> = body[..colon]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if processes.is_empty() {
                return Err(format!("no process names in: {clause}"));
            }
            let action = parse_action(body[colon + 1..].trim())?;
            set.overrides.push(Override { processes, action });
            continue;
        }

        return Err(format!("unrecognised clause: {clause}"));
    }

    Ok(set)
}

fn strip_comment(s: &str) -> &str {
    match s.find('#') {
        Some(i) => &s[..i],
        None => s,
    }
}

fn parse_action(s: &str) -> Result<Action, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty action".into());
    }

    // Forward the binding's own key. Accept both the sigil and bare spellings,
    // plus a `kitty:` prefix that forces the kitty-protocol encoding.
    if s == "$source" || s == "source" {
        return Ok(Action::Source { kitty: false });
    }
    if s == "kitty:$source" || s == "kitty:source" {
        return Ok(Action::Source { kitty: true });
    }

    // Friendly key notation: `key ctrl+j`, `key alt+enter`, `key f5`, … The
    // spec is decoded to bytes up front, so it behaves exactly like `write`
    // from there on. `send` is an alias.
    if let Some(rest) = s.strip_prefix("key ").or_else(|| s.strip_prefix("send ")) {
        let bytes = crate::keyspec::parse(rest.trim())?;
        return Ok(Action::Write(bytes));
    }
    if s == "key" || s == "send" {
        return Err("'key' requires a key spec (e.g. `key ctrl+j`)".into());
    }

    if let Some(rest) = s.strip_prefix("write ") {
        return Ok(Action::Write(unescape(rest.trim_start())));
    }
    if s == "write" {
        return Ok(Action::Write(Vec::new()));
    }

    if let Some(rest) = s.strip_prefix("do ") {
        let mut toks = rest.split_whitespace();
        let name = toks
            .next()
            .ok_or_else(|| "'do' requires an action name".to_string())?
            .to_string();
        let args: Vec<String> = toks.map(|t| t.to_string()).collect();
        return Ok(Action::Do { name, args });
    }

    if let Some(rest) = s.strip_prefix("pipe ") {
        return parse_pipe_action(rest.trim());
    }
    if s == "pipe" {
        return Err("'pipe' requires a target URL".into());
    }

    if let Some(rest) = s.strip_prefix("run ") {
        let argv: Vec<String> = rest.split_whitespace().map(|t| t.to_string()).collect();
        if argv.is_empty() {
            return Err("'run' requires a command".into());
        }
        return Ok(Action::Run { argv });
    }
    if s == "run" {
        return Err("'run' requires a command".into());
    }

    if s == "noop" || s == "nothing" || s == "swallow" {
        return Ok(Action::Noop);
    }

    Err(format!("unknown action: '{s}'"))
}

fn parse_pipe_action(rest: &str) -> Result<Action, String> {
    let mut tokens = rest.split_whitespace();
    let url = tokens
        .next()
        .map(str::to_string)
        .ok_or_else(|| "'pipe' requires a target URL".to_string())?;

    let mut name: Option<String> = None;
    let mut payload: Option<String> = None;
    let mut config: BTreeMap<String, String> = BTreeMap::new();

    for tok in tokens {
        let (key, value) = tok
            .split_once('=')
            .ok_or_else(|| format!("expected key=value in 'pipe', got: '{tok}'"))?;
        if key.is_empty() {
            return Err(format!("empty key in 'pipe': '{tok}'"));
        }
        match key {
            "name" => name = Some(value.to_string()),
            "payload" => payload = Some(value.to_string()),
            other => {
                config.insert(other.to_string(), value.to_string());
            }
        }
    }

    Ok(Action::Pipe {
        url,
        name,
        payload,
        config,
    })
}

/// Decode common backslash escapes into raw bytes.
pub fn unescape(s: &str) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            push_char(&mut out, c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('0') => out.push(0),
            Some('a') => out.push(0x07),
            Some('b') => out.push(0x08),
            Some('f') => out.push(0x0c),
            Some('v') => out.push(0x0b),
            Some('e' | 'E') => out.push(0x1b),
            Some('\\') => out.push(b'\\'),
            Some('"') => out.push(b'"'),
            Some('\'') => out.push(b'\''),
            Some(' ') => out.push(b' '),
            Some('x') => {
                if let (Some(a), Some(b)) = (chars.next(), chars.next()) {
                    if let Some(byte) = hex_pair(a, b) {
                        out.push(byte);
                    }
                }
            }
            Some('u') => {
                // \u{NNNN} — Unicode codepoint.
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut hex = String::new();
                    while let Some(&p) = chars.peek() {
                        if p == '}' {
                            chars.next();
                            break;
                        }
                        hex.push(p);
                        chars.next();
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            push_char(&mut out, ch);
                        }
                    }
                }
            }
            Some(other) => {
                out.push(b'\\');
                push_char(&mut out, other);
            }
            None => out.push(b'\\'),
        }
    }
    out
}

fn push_char(out: &mut Vec<u8>, c: char) {
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

fn hex_pair(a: char, b: char) -> Option<u8> {
    let h = a.to_digit(16)? as u8;
    let l = b.to_digit(16)? as u8;
    Some(h * 16 + l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_only() {
        let r = parse("default: write \\r").unwrap();
        assert!(matches!(r.default, Some(Action::Write(ref b)) if b == b"\r"));
        assert!(r.overrides.is_empty());
    }

    #[test]
    fn parses_default_and_overrides() {
        let r = parse(
            "default: write \\r\n\
             when agent: write \\x1b\\r\n\
             when fzf,less: do move_focus down",
        )
        .unwrap();
        assert!(matches!(r.default, Some(Action::Write(_))));
        assert_eq!(r.overrides.len(), 2);
        assert_eq!(r.overrides[0].processes, vec!["agent".to_string()]);
        assert_eq!(
            r.overrides[1].processes,
            vec!["fzf".to_string(), "less".to_string()]
        );
    }

    #[test]
    fn supports_semicolons() {
        let r = parse("default: write \\n ; when fzf: write \\x0a").unwrap();
        assert!(r.default.is_some());
        assert_eq!(r.overrides.len(), 1);
    }

    #[test]
    fn unescape_handles_common_sequences() {
        assert_eq!(unescape("\\r"), b"\r");
        assert_eq!(unescape("\\x1b\\r"), b"\x1b\r");
        assert_eq!(unescape("\\e[A"), b"\x1b[A");
        assert_eq!(unescape("plain"), b"plain");
        assert_eq!(unescape("\\u{1b}"), b"\x1b");
    }

    #[test]
    fn choose_prefers_override() {
        let r = parse("default: write a\nwhen agent: write b").unwrap();
        assert!(matches!(r.choose(&["agent"]), Action::Write(ref b) if b == b"b"));
        assert!(matches!(r.choose(&["zsh"]), Action::Write(ref b) if b == b"a"));
        assert!(matches!(r.choose(&[]), Action::Write(ref b) if b == b"a"));
    }

    #[test]
    fn choose_is_case_insensitive() {
        let r = parse("default: noop\nwhen Agent: write x").unwrap();
        assert!(matches!(r.choose(&["agent"]), Action::Write(_)));
        assert!(matches!(r.choose(&["AGENT"]), Action::Write(_)));
    }

    #[test]
    fn choose_matches_any_candidate() {
        // The immediate fg process is `zsh`, but `fzf` is in the descendant
        // set — the `when fzf` clause should still win.
        let r = parse("default: do move_focus down\nwhen fzf: write \\n").unwrap();
        let pick = r.choose(&["zsh", "fzf"]);
        match pick {
            Action::Write(ref b) => assert_eq!(b, b"\n"),
            other => panic!("expected Write, got {other:?}"),
        }
    }

    #[test]
    fn comments_are_stripped() {
        let r = parse("# header comment\ndefault: write \\r # trailing").unwrap();
        assert!(r.default.is_some());
    }

    #[test]
    fn noop_action() {
        let r = parse("default: noop").unwrap();
        assert!(matches!(r.default, Some(Action::Noop)));
    }

    #[test]
    fn do_action_with_args() {
        let r = parse("default: do move_focus down").unwrap();
        match r.default.unwrap() {
            Action::Do { name, args } => {
                assert_eq!(name, "move_focus");
                assert_eq!(args, vec!["down".to_string()]);
            }
            _ => panic!("expected Do"),
        }
    }

    #[test]
    fn pipe_action_parses_url_only() {
        let r = parse("default: pipe https://example.com/plugin.wasm").unwrap();
        match r.default.unwrap() {
            Action::Pipe {
                url,
                name,
                payload,
                config,
            } => {
                assert_eq!(url, "https://example.com/plugin.wasm");
                assert!(name.is_none());
                assert!(payload.is_none());
                assert!(config.is_empty());
            }
            _ => panic!("expected Pipe"),
        }
    }

    #[test]
    fn pipe_action_parses_name_payload_and_config() {
        let r = parse(
            "default: pipe https://x/p.wasm name=move_focus payload=down \
             move_mod=ctrl use_arrow_keys=false",
        )
        .unwrap();
        match r.default.unwrap() {
            Action::Pipe {
                url,
                name,
                payload,
                config,
            } => {
                assert_eq!(url, "https://x/p.wasm");
                assert_eq!(name.as_deref(), Some("move_focus"));
                assert_eq!(payload.as_deref(), Some("down"));
                assert_eq!(config.get("move_mod").map(String::as_str), Some("ctrl"));
                assert_eq!(
                    config.get("use_arrow_keys").map(String::as_str),
                    Some("false")
                );
                assert!(!config.contains_key("name"));
                assert!(!config.contains_key("payload"));
            }
            _ => panic!("expected Pipe"),
        }
    }

    #[test]
    fn key_action_decodes_to_bytes() {
        let r = parse("default: key ctrl+j").unwrap();
        assert!(matches!(r.default, Some(Action::Write(ref b)) if b == &[0x0a]));

        let r = parse("when cursor-agent: key alt+enter").unwrap();
        assert!(matches!(r.overrides[0].action, Action::Write(ref b) if b == b"\x1b\r"));
    }

    #[test]
    fn send_is_an_alias_for_key() {
        let r = parse("default: send esc").unwrap();
        assert!(matches!(r.default, Some(Action::Write(ref b)) if b == &[0x1b]));
    }

    #[test]
    fn key_action_propagates_parse_errors() {
        assert!(parse("default: key frobnicate").is_err());
        assert!(parse("default: key").is_err());
    }

    #[test]
    fn source_action_parses() {
        assert!(matches!(
            parse("default: $source").unwrap().default,
            Some(Action::Source { kitty: false })
        ));
        assert!(matches!(
            parse("default: source").unwrap().default,
            Some(Action::Source { kitty: false })
        ));
        let r = parse("default: do move_focus down; when fzf: $source").unwrap();
        assert!(matches!(
            r.overrides[0].action,
            Action::Source { kitty: false }
        ));
    }

    #[test]
    fn kitty_source_action_parses() {
        assert!(matches!(
            parse("default: kitty:$source").unwrap().default,
            Some(Action::Source { kitty: true })
        ));
        assert!(matches!(
            parse("default: kitty:source").unwrap().default,
            Some(Action::Source { kitty: true })
        ));
    }

    #[test]
    fn run_action_parses_command_and_args() {
        let r = parse("default: run /usr/local/bin/copy-pwd {pid} extra").unwrap();
        match r.default.unwrap() {
            Action::Run { argv } => {
                assert_eq!(
                    argv,
                    vec![
                        "/usr/local/bin/copy-pwd".to_string(),
                        "{pid}".to_string(),
                        "extra".to_string()
                    ]
                );
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn run_action_requires_a_command() {
        assert!(parse("default: run").is_err());
    }

    #[test]
    fn run_can_coexist_with_when_clauses() {
        let r = parse("default: run /bin/copy-pwd {pid}; when fzf: noop").unwrap();
        assert!(matches!(r.default, Some(Action::Run { .. })));
        assert_eq!(r.overrides.len(), 1);
        assert!(matches!(r.overrides[0].action, Action::Noop));
    }

    #[test]
    fn pipe_action_rejects_token_without_equals() {
        let err = parse("default: pipe URL bogus_token").unwrap_err();
        assert!(err.contains("key=value"));
    }

    #[test]
    fn pipe_action_keeps_extra_equals_in_value() {
        let r = parse("default: pipe URL payload=foo=bar=baz").unwrap();
        match r.default.unwrap() {
            Action::Pipe { payload, .. } => {
                assert_eq!(payload.as_deref(), Some("foo=bar=baz"));
            }
            _ => panic!("expected Pipe"),
        }
    }

    #[test]
    fn pipe_can_coexist_with_when_clauses() {
        let r = parse(
            "default: pipe https://x/p.wasm name=move_focus payload=down; \
             when fzf: write \\n",
        )
        .unwrap();
        assert!(matches!(r.default, Some(Action::Pipe { .. })));
        assert_eq!(r.overrides.len(), 1);
        assert!(matches!(r.overrides[0].action, Action::Write(_)));
    }
}
