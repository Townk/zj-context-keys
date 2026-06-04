//! Action executor.
//!
//! `Write` sends raw bytes to the focused pane (requires `WriteToStdin`).
//! `Do`    invokes a built-in Zellij action by name (requires
//!         `ChangeApplicationState`).

use zellij_tile::prelude::*;
use zj_context_keys::parser::Action;

pub fn execute(action: Action) {
    match action {
        Action::Noop => {}
        Action::Write(bytes) => {
            if !bytes.is_empty() {
                write(bytes);
            }
        }
        Action::Do { name, args } => run_zellij_action(&name, &args),
        Action::Pipe {
            url,
            name,
            payload,
            config,
        } => run_pipe_action(url, name, payload, config),
        // `Run` and `Source` are intercepted in main.rs `handle_pipe` before
        // reaching here: `Run` needs the focused pane's PID/id, and `Source`
        // needs the dispatching message's `name` (the source key spec) — both
        // of which only the stateful plugin side can resolve.
        Action::Run { .. } | Action::Source { .. } => {}
    }
}

fn run_pipe_action(
    url: String,
    name: Option<String>,
    payload: Option<String>,
    config: std::collections::BTreeMap<String, String>,
) {
    // Plugin pipes are identified by a `message_name`; use the caller's
    // `name=...` if supplied, otherwise a stable default so the receiver
    // can still match on it if it wants.
    let msg_name = name.unwrap_or_else(|| "context-keys-pipe".to_string());
    let mut msg = MessageToPlugin::new(msg_name).with_plugin_url(url);
    if let Some(p) = payload {
        msg = msg.with_payload(p);
    }
    if !config.is_empty() {
        msg = msg.with_plugin_config(config);
    }
    pipe_message_to_plugin(msg);
}

fn run_zellij_action(name: &str, args: &[String]) {
    let canon = name.to_lowercase().replace('-', "_");
    match canon.as_str() {
        "move_focus" | "movefocus" => {
            if let Some(dir) = arg_direction(args, 0) {
                move_focus(dir);
            }
        }
        "move_focus_or_tab" | "movefocusortab" => {
            if let Some(dir) = arg_direction(args, 0) {
                move_focus_or_tab(dir);
            }
        }
        "move_pane" | "movepane" => match arg_direction(args, 0) {
            Some(dir) => move_pane_with_direction(dir),
            None => move_pane(),
        },
        "new_tab" | "newtab" => {
            let name = args.first().cloned();
            new_tab(name, None::<String>);
        }
        "go_to_next_tab" | "gotonexttab" => go_to_next_tab(),
        "go_to_previous_tab" | "gotoprevioustab" | "go_to_prev_tab" => go_to_previous_tab(),
        "go_to_tab" | "gototab" => {
            if let Some(n) = args.first().and_then(|s| s.parse::<u32>().ok()) {
                go_to_tab(n);
            }
        }
        "go_to_tab_name" | "gototabname" => {
            if let Some(n) = args.first() {
                go_to_tab_name(n);
            }
        }
        "close_focus" | "closefocus" | "close_pane" | "closepane" => close_focus(),
        "close_tab" | "closetab" | "close_focused_tab" => close_focused_tab(),
        "toggle_pane_embed_or_eject" | "toggle_floating" | "togglefloatingpanes" => {
            toggle_pane_embed_or_eject()
        }
        "toggle_pane_frames" | "togglepaneframes" => toggle_pane_frames(),
        "toggle_fullscreen" | "togglefullscreen" | "toggle_focus_fullscreen" => {
            toggle_focus_fullscreen()
        }
        "switch_mode" | "switchmode" | "switch_to_mode" => {
            if let Some(mode) = args.first().and_then(parse_mode) {
                switch_to_input_mode(&mode);
            }
        }
        "detach" => detach(),
        "quit" | "quit_zellij" => quit_zellij(),
        "write_chars" | "writechars" => {
            if let Some(s) = args.first() {
                write_chars(s);
            }
        }
        _ => {
            eprintln!("zj-context-keys: unknown 'do' action: {name}");
        }
    }
}

fn arg_direction(args: &[String], i: usize) -> Option<Direction> {
    args.get(i).and_then(|s| parse_direction(s))
}

fn parse_direction(s: &str) -> Option<Direction> {
    match s.to_lowercase().as_str() {
        "down" => Some(Direction::Down),
        "up" => Some(Direction::Up),
        "left" => Some(Direction::Left),
        "right" => Some(Direction::Right),
        _ => None,
    }
}

fn parse_mode(s: &String) -> Option<InputMode> {
    match s.to_lowercase().as_str() {
        "normal" => Some(InputMode::Normal),
        "locked" => Some(InputMode::Locked),
        "resize" => Some(InputMode::Resize),
        "pane" => Some(InputMode::Pane),
        "tab" => Some(InputMode::Tab),
        "scroll" => Some(InputMode::Scroll),
        "entersearch" | "enter_search" => Some(InputMode::EnterSearch),
        "search" => Some(InputMode::Search),
        "renametab" | "rename_tab" => Some(InputMode::RenameTab),
        "renamepane" | "rename_pane" => Some(InputMode::RenamePane),
        "session" => Some(InputMode::Session),
        "move" => Some(InputMode::Move),
        "prompt" => Some(InputMode::Prompt),
        "tmux" => Some(InputMode::Tmux),
        _ => None,
    }
}
