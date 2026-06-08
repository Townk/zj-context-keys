//! zj-context-keys — context-aware keybindings for Zellij.
//!
//! The plugin is meant to be loaded headless at session start (via
//! `load_plugins` in the Zellij config). User keybindings dispatch into it
//! through `MessagePlugin`, and the plugin chooses what to do based on the
//! foreground *process tree* of the focused terminal pane.
//!
//! Process-tree matching matters because shells and TUIs often invoke
//! programs in ways that don't change the pty's foreground process group
//! (e.g. zsh tab completion launching `fzf`, or a shell function running it
//! via command substitution). In those cases `get_pane_running_command`
//! keeps returning the shell name, so we also keep a per-pane set of
//! descendant process basenames refreshed by a periodic `ps` poll.
//!
//! Each pipe payload is parsed as a small rule DSL — see `parser.rs`.

mod action;

use std::collections::{BTreeMap, HashMap, HashSet};

use zellij_tile::prelude::*;

use zj_context_keys::{basename, compute_descendants, keyspec, parser::parse, parser::Action};

register_plugin!(State);

/// How often to refresh the focused pane's descendant process set.
const POLL_INTERVAL_S: f64 = 0.5;

const CTX_KEY: &str = "context_keys_kind";
const CTX_PS: &str = "ps";
const CTX_EXEC: &str = "exec";
const CTX_PANE_ID: &str = "pane_id";

#[derive(Default)]
struct State {
    permissions_granted: bool,
    permissions_denied: bool,
    /// Set once we've logged the first keybind dropped for lack of
    /// permissions, so we don't spam stderr on every subsequent press.
    dropped_keybind_warned: bool,
    timer_started: bool,

    /// PID of each terminal pane's root process (typically the shell).
    pane_pid: HashMap<u32, i32>,
    /// Basenames of every descendant process for each terminal pane.
    pane_descendants: HashMap<u32, HashSet<String>>,
    /// At most one `ps` refresh in flight at a time to avoid storming.
    refresh_in_flight: bool,
}

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::WriteToStdin,
            PermissionType::RunCommands,
            PermissionType::MessageAndLaunchOtherPlugins,
        ]);
        subscribe(&[
            EventType::PermissionRequestResult,
            EventType::Timer,
            EventType::RunCommandResult,
        ]);
        set_selectable(false);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                self.permissions_granted = true;
                self.ensure_timer();
                // Kick a refresh immediately so the first keystroke after
                // permission grant doesn't see an empty descendant cache.
                self.refresh_focused_pane_tree();
            }
            Event::PermissionRequestResult(PermissionStatus::Denied) => {
                // Without permissions the plugin can't read pane state or
                // dispatch actions, so mark it disabled. `pipe()` will drop
                // any incoming keybinds (logging once) rather than queue them.
                self.permissions_denied = true;
                eprintln!("zj-context-keys: permissions denied; plugin is disabled");
            }
            Event::Timer(_) => {
                if self.permissions_granted {
                    self.refresh_focused_pane_tree();
                }
                set_timeout(POLL_INTERVAL_S);
            }
            Event::RunCommandResult(exit_code, stdout, _stderr, context)
                if context.get(CTX_KEY).map(String::as_str) == Some(CTX_PS) =>
            {
                self.refresh_in_flight = false;
                if exit_code == Some(0) {
                    if let Some(id) = context.get(CTX_PANE_ID).and_then(|s| s.parse::<u32>().ok()) {
                        if let Some(&root_pid) = self.pane_pid.get(&id) {
                            let descendants = compute_descendants(&stdout, root_pid);
                            self.pane_descendants.insert(id, descendants);
                        }
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn pipe(&mut self, msg: PipeMessage) -> bool {
        // We only dispatch rules sent by user keybinds / the CLI. Plugin-to-
        // plugin broadcasts (e.g. zj-hud's `__zj_hud_sync_state`) are delivered
        // to every loaded plugin; parsing their payloads as rules just spams
        // "bad rule" and risks flooding Zellij's plugin stderr buffer. Drop
        // them before any parsing or logging.
        if matches!(msg.source, PipeSource::Plugin(_)) {
            return false;
        }

        // Keybind messages are time-sensitive: a press maps to whatever pane
        // is focused *now*. A `PipeSource::Keybind` carries no originating
        // pane id, so a message queued before permissions were granted can't
        // be replayed against the right context later. Drop it instead.
        if !self.permissions_granted {
            if !self.dropped_keybind_warned {
                self.dropped_keybind_warned = true;
                if self.permissions_denied {
                    eprintln!(
                        "zj-context-keys: permissions denied; ignoring keybind '{}'",
                        msg.name
                    );
                } else {
                    eprintln!(
                        "zj-context-keys: keybind '{}' received before permissions were granted; ignoring",
                        msg.name
                    );
                }
            }
            return false;
        }
        self.handle_pipe(msg);
        false
    }

    fn render(&mut self, _rows: usize, _cols: usize) {}
}

impl State {
    fn ensure_timer(&mut self) {
        if !self.timer_started {
            self.timer_started = true;
            set_timeout(POLL_INTERVAL_S);
        }
    }

    fn handle_pipe(&mut self, msg: PipeMessage) {
        let payload = match msg.payload {
            Some(p) if !p.trim().is_empty() => p,
            _ => return,
        };

        let rules = match parse(&payload) {
            Ok(r) => r,
            Err(err) => {
                eprintln!("zj-context-keys: bad rule for '{}': {}", msg.name, err);
                return;
            }
        };

        // Build the candidate set: immediate fg command + every descendant.
        let mut owned: Vec<String> = Vec::new();
        let mut focused_id: Option<u32> = None;

        if let Ok((_tab, pane_id)) = get_focused_pane_info() {
            if let PaneId::Terminal(id) = pane_id {
                focused_id = Some(id);
                if let Ok(argv) = get_pane_running_command(pane_id) {
                    if let Some(exe) = argv.first() {
                        let trimmed = exe.trim();
                        if !trimmed.is_empty() {
                            owned.push(basename(trimmed));
                        }
                    }
                }
            }
        }

        if let Some(id) = focused_id {
            if let Some(descendants) = self.pane_descendants.get(&id) {
                owned.extend(descendants.iter().cloned());
            }
        }

        let candidates: Vec<&str> = owned.iter().map(String::as_str).collect();
        let action = rules.choose(&candidates);
        match action {
            // `run` needs the focused pane's PID/id for placeholder
            // substitution, which lives on the stateful plugin side, so
            // handle it here rather than in the stateless action executor.
            Action::Run { argv } => self.run_external(argv, focused_id),
            // `$source` forwards the binding's own key: interpret the
            // dispatching message's `name` as a key spec and send its bytes.
            // `kitty:$source` forces the kitty (CSI-u) encoding by prefixing
            // the spec, unless `name` already carries the prefix itself.
            Action::Source { kitty } => {
                let spec = if kitty && !msg.name.trim_start().starts_with("kitty:") {
                    format!("kitty:{}", msg.name)
                } else {
                    msg.name.clone()
                };
                match keyspec::parse(&spec) {
                    Ok(bytes) => action::execute(Action::Write(bytes)),
                    Err(err) => eprintln!(
                        "zj-context-keys: $source needs `name` to be a key spec, but '{}' isn't one: {}",
                        msg.name, err
                    ),
                }
            }
            other => action::execute(other),
        }

        // Cheap opportunistic refresh so the next keystroke benefits from
        // any process churn caused by *this* one.
        if !self.refresh_in_flight {
            self.refresh_focused_pane_tree();
        }
    }

    /// Execute an external command headlessly (no pane, no flash).
    ///
    /// Placeholders in `argv` are substituted against the focused terminal
    /// pane: `{pid}` → its root process PID, `{pane_id}` → its numeric id.
    /// The PID lets a helper resolve the pane's cwd via `lsof` without
    /// relying on ZELLIJ_* env vars (the host doesn't set them for
    /// run_command children). If a `{pid}` placeholder is present but the
    /// PID can't be resolved, we bail rather than run the command with a
    /// stale/empty value.
    fn run_external(&mut self, argv: Vec<String>, focused_id: Option<u32>) {
        let needs_pid = argv.iter().any(|a| a.contains("{pid}"));
        let needs_pane_id = argv.iter().any(|a| a.contains("{pane_id}"));

        let pid: Option<i32> = if needs_pid {
            match focused_id {
                Some(id) => match self.pane_pid.get(&id) {
                    Some(&p) => Some(p),
                    None => match get_pane_pid(PaneId::Terminal(id)) {
                        Ok(p) => {
                            self.pane_pid.insert(id, p);
                            Some(p)
                        }
                        Err(_) => None,
                    },
                },
                None => None,
            }
        } else {
            None
        };

        if needs_pid && pid.is_none() {
            eprintln!("zj-context-keys: run: could not resolve {{pid}} for focused pane");
            return;
        }

        let resolved: Vec<String> = argv
            .into_iter()
            .map(|tok| {
                let mut t = tok;
                if let Some(p) = pid {
                    t = t.replace("{pid}", &p.to_string());
                }
                if needs_pane_id {
                    if let Some(id) = focused_id {
                        t = t.replace("{pane_id}", &id.to_string());
                    }
                }
                t
            })
            .collect();

        let argv_ref: Vec<&str> = resolved.iter().map(String::as_str).collect();
        let mut ctx = BTreeMap::new();
        ctx.insert(CTX_KEY.to_string(), CTX_EXEC.to_string());
        run_command(&argv_ref, ctx);
    }

    fn refresh_focused_pane_tree(&mut self) {
        if self.refresh_in_flight {
            return;
        }
        let (_, pane_id) = match get_focused_pane_info() {
            Ok(v) => v,
            Err(_) => return,
        };
        let id = match pane_id {
            PaneId::Terminal(id) => id,
            _ => return,
        };

        if let std::collections::hash_map::Entry::Vacant(entry) = self.pane_pid.entry(id) {
            match get_pane_pid(pane_id) {
                Ok(pid) => {
                    entry.insert(pid);
                }
                Err(_) => return,
            }
        }

        self.refresh_in_flight = true;
        let mut ctx = BTreeMap::new();
        ctx.insert(CTX_KEY.to_string(), CTX_PS.to_string());
        ctx.insert(CTX_PANE_ID.to_string(), id.to_string());
        run_command(&["ps", "-A", "-o", "pid=,ppid=,comm="], ctx);
    }
}
