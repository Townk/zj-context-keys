//! Library side of zj-context-keys: the rule DSL parser.
//!
//! The binary (`src/main.rs`) wires this into the Zellij plugin runtime, but
//! everything host-agnostic lives here so it can be unit-tested on the host
//! toolchain without dragging in the WASM-only `zellij-tile` shim symbols.

pub mod keyspec;
pub mod parser;

use std::collections::{HashMap, HashSet};

/// Return the last `/`-separated component of `path` (the executable name).
pub fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Parse the output of `ps -A -o pid=,ppid=,comm=` and return the basenames
/// of every descendant process of `root_pid` (the root pid itself is not
/// included).
pub fn compute_descendants(stdout: &[u8], root_pid: i32) -> HashSet<String> {
    let text = String::from_utf8_lossy(stdout);

    // pid -> (ppid, comm). `comm` should be a single token but we join the
    // remainder of the line just in case a binary name contains spaces.
    let mut info: HashMap<i32, (i32, String)> = HashMap::with_capacity(256);
    for line in text.lines() {
        let mut iter = line.split_whitespace();
        let pid = match iter.next().and_then(|s| s.parse::<i32>().ok()) {
            Some(v) => v,
            None => continue,
        };
        let ppid = match iter.next().and_then(|s| s.parse::<i32>().ok()) {
            Some(v) => v,
            None => continue,
        };
        let rest: Vec<&str> = iter.collect();
        if rest.is_empty() {
            continue;
        }
        info.insert(pid, (ppid, rest.join(" ")));
    }

    let mut children: HashMap<i32, Vec<i32>> = HashMap::with_capacity(info.len());
    for (&pid, (ppid, _)) in &info {
        children.entry(*ppid).or_default().push(pid);
    }

    let mut out: HashSet<String> = HashSet::new();
    let mut stack: Vec<i32> = vec![root_pid];
    while let Some(pid) = stack.pop() {
        if let Some(kids) = children.get(&pid) {
            for &child in kids {
                if let Some((_, comm)) = info.get(&child) {
                    let name = basename(comm.trim());
                    if !name.is_empty() {
                        out.insert(name);
                    }
                }
                stack.push(child);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_handles_paths() {
        assert_eq!(basename("agent"), "agent");
        assert_eq!(basename("/usr/local/bin/agent"), "agent");
        assert_eq!(basename("/usr/bin/"), "");
        assert_eq!(basename("cursor-agent"), "cursor-agent");
    }

    #[test]
    fn compute_descendants_collects_full_subtree() {
        // Tree:
        //   100 (zsh)
        //   ├── 200 (find)
        //   ├── 201 (fzf)         ← direct child
        //   └── 202 (sh)
        //       └── 300 (fzf)     ← grandchild
        // Unrelated:
        //   999 (launchd)
        let ps = b"100 1 zsh\n\
                   200 100 find\n\
                   201 100 fzf\n\
                   202 100 sh\n\
                   300 202 fzf\n\
                   999 1 launchd\n";
        let d = compute_descendants(ps, 100);
        assert!(d.contains("zsh") == false, "root pid itself excluded");
        assert!(d.contains("find"));
        assert!(d.contains("fzf"));
        assert!(d.contains("sh"));
        assert!(!d.contains("launchd"));
    }

    #[test]
    fn compute_descendants_dedupes_basenames() {
        // Two `fzf` processes at different depths collapse into one entry.
        let ps = b"100 1 zsh\n\
                   200 100 /opt/homebrew/bin/fzf\n\
                   300 200 fzf\n";
        let d = compute_descendants(ps, 100);
        assert_eq!(d.iter().filter(|n| n.as_str() == "fzf").count(), 1);
    }

    #[test]
    fn compute_descendants_handles_empty_input() {
        let d = compute_descendants(b"", 100);
        assert!(d.is_empty());
    }

    #[test]
    fn compute_descendants_handles_orphan_root() {
        let ps = b"999 1 launchd\n";
        let d = compute_descendants(ps, 12345);
        assert!(d.is_empty());
    }
}
