//! Friendly key notation → terminal byte sequences.
//!
//! Instead of writing raw bytes (`write \x1b\r`, `write \x0a`), a rule can
//! describe a keystroke the way you'd say it out loud:
//!
//!   key ctrl+j        # → 0x0a
//!   key alt+enter     # → ESC CR  (\x1b\r)
//!   key f5            # → \e[15~
//!   key shift+tab     # → \e[Z    (backtab)
//!
//! Grammar: zero or more modifiers followed by a base key, joined with `+`
//! (or `-`), case-insensitively:
//!
//!   [<mod>+]* <base>
//!
//! Modifiers: `ctrl`, `alt` (a.k.a. `opt`/`option`/`meta`), `shift`,
//! `super` (a.k.a. `cmd`/`command`). Base keys are a single character
//! (`a`, `/`, `5`, …) or a named key (`enter`, `tab`, `esc`, `space`,
//! `backspace`, `delete`, `up`/`down`/`left`/`right`, `home`, `end`,
//! `pageup`/`pagedown`, `insert`, `f1`–`f12`).
//!
//! Encoding is the *legacy* terminal convention by default. A handful of
//! keys (most famously `shift+enter`) have no distinct legacy encoding —
//! they only differ from the unmodified key when the application speaks the
//! kitty keyboard protocol. Prefix the spec with `kitty:` to emit the
//! CSI-u form for those cases:
//!
//!   key kitty:shift+enter   # → \e[13;2u

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Modifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    sup: bool,
}

impl Modifiers {
    /// xterm/kitty modifier code: 1 + bitfield. Used both for CSI parameter
    /// encoding (`\e[1;<code>A`) and, minus one, as the kitty modifier param.
    fn xterm_code(self) -> u8 {
        1 + (self.shift as u8)
            + ((self.alt as u8) << 1)
            + ((self.ctrl as u8) << 2)
            + ((self.sup as u8) << 3)
    }
}

/// Parse a key spec into the bytes a terminal would deliver for that key.
pub fn parse(spec: &str) -> Result<Vec<u8>, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("empty key spec".into());
    }

    let (kitty, body) = match spec.strip_prefix("kitty:") {
        Some(rest) => (true, rest.trim()),
        None => (false, spec),
    };
    if body.is_empty() {
        return Err(format!("empty key spec: '{spec}'"));
    }

    let (mods, base) = split_spec(body)?;

    if kitty {
        encode_kitty(&base, mods, spec)
    } else {
        encode_legacy(&base, mods, spec)
    }
}

/// Split `body` into its modifiers and the trailing base key. Handles a
/// literal `+` base key written as a doubled separator (e.g. `ctrl++`).
fn split_spec(body: &str) -> Result<(Modifiers, String), String> {
    // Allow `-` as a separator too, but never split a lone `-` base key.
    let normalized = if body == "-" { "-".to_string() } else { body.replace('-', "+") };

    let tokens: Vec<&str> = normalized.split('+').collect();
    let (mod_tokens, base): (&[&str], String) =
        if tokens.last() == Some(&"") && tokens.len() >= 2 {
            // Trailing empty token ⇒ the base key is a literal `+`.
            (&tokens[..tokens.len() - 2], "+".to_string())
        } else {
            (&tokens[..tokens.len() - 1], tokens[tokens.len() - 1].to_string())
        };

    let mut mods = Modifiers::default();
    for tok in mod_tokens {
        let t = tok.trim().to_lowercase();
        match t.as_str() {
            "" => continue,
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "opt" | "option" | "meta" => mods.alt = true,
            "shift" => mods.shift = true,
            "super" | "cmd" | "command" | "win" | "hyper" => mods.sup = true,
            other => return Err(format!("unknown modifier '{other}' in key spec '{body}'")),
        }
    }

    if base.trim().is_empty() {
        return Err(format!("missing base key in key spec '{body}'"));
    }
    Ok((mods, base))
}

/// One of the named keys that maps onto a terminal escape sequence whose
/// modified form follows the xterm `;<code>` convention.
enum Named {
    /// CSI sequence ending in a letter, e.g. Up = `\e[A`.
    CsiLetter(u8),
    /// CSI sequence with a numeric parameter, e.g. Delete = `\e[3~`.
    CsiTilde(u8),
    /// SS3 function key (F1–F4), e.g. F1 = `\eOP`.
    Ss3(u8),
}

fn named_key(base: &str) -> Option<Named> {
    Some(match base.to_lowercase().as_str() {
        "up" => Named::CsiLetter(b'A'),
        "down" => Named::CsiLetter(b'B'),
        "right" => Named::CsiLetter(b'C'),
        "left" => Named::CsiLetter(b'D'),
        "home" => Named::CsiLetter(b'H'),
        "end" => Named::CsiLetter(b'F'),
        "insert" | "ins" => Named::CsiTilde(2),
        "delete" | "del" => Named::CsiTilde(3),
        "pageup" | "pgup" | "prior" => Named::CsiTilde(5),
        "pagedown" | "pgdn" | "next" => Named::CsiTilde(6),
        "f1" => Named::Ss3(b'P'),
        "f2" => Named::Ss3(b'Q'),
        "f3" => Named::Ss3(b'R'),
        "f4" => Named::Ss3(b'S'),
        "f5" => Named::CsiTilde(15),
        "f6" => Named::CsiTilde(17),
        "f7" => Named::CsiTilde(18),
        "f8" => Named::CsiTilde(19),
        "f9" => Named::CsiTilde(20),
        "f10" => Named::CsiTilde(21),
        "f11" => Named::CsiTilde(23),
        "f12" => Named::CsiTilde(24),
        _ => return None,
    })
}

fn encode_legacy(base: &str, mods: Modifiers, spec: &str) -> Result<Vec<u8>, String> {
    // Escape-sequence keys (arrows, nav, function keys) fold every modifier
    // into the standard xterm `;<code>` parameter.
    if let Some(named) = named_key(base) {
        let code = mods.xterm_code();
        let seq = match named {
            Named::CsiLetter(l) => {
                if code == 1 {
                    format!("\x1b[{}", l as char)
                } else {
                    format!("\x1b[1;{}{}", code, l as char)
                }
            }
            Named::CsiTilde(n) => {
                if code == 1 {
                    format!("\x1b[{n}~")
                } else {
                    format!("\x1b[{n};{code}~")
                }
            }
            Named::Ss3(l) => {
                if code == 1 {
                    format!("\x1bO{}", l as char)
                } else {
                    format!("\x1b[1;{}{}", code, l as char)
                }
            }
        };
        return Ok(seq.into_bytes());
    }

    // The remaining keys are "simple": a base byte (or short sequence) with
    // ctrl/shift applied directly and alt prepending ESC.
    let lower = base.to_lowercase();
    let mut bytes: Vec<u8> = match lower.as_str() {
        "enter" | "return" | "cr" => vec![b'\r'],
        "tab" => {
            if mods.shift {
                // Backtab. Other modifiers have no legacy encoding here.
                return Ok(b"\x1b[Z".to_vec());
            }
            vec![b'\t']
        }
        "esc" | "escape" => vec![0x1b],
        "space" | "spc" => {
            if mods.ctrl {
                vec![0x00]
            } else {
                vec![b' ']
            }
        }
        "backspace" | "bs" => {
            if mods.ctrl {
                vec![0x08]
            } else {
                vec![0x7f]
            }
        }
        _ => {
            // A single character (letter, digit, punctuation).
            let chars: Vec<char> = base.chars().collect();
            if chars.len() != 1 {
                return Err(format!("unknown key '{base}' in key spec '{spec}'"));
            }
            let mut ch = chars[0];
            if mods.shift && ch.is_ascii_alphabetic() {
                ch = ch.to_ascii_uppercase();
            }
            if mods.ctrl {
                control_byte(ch)
                    .map(|b| vec![b])
                    .ok_or_else(|| format!("ctrl+{ch} has no terminal encoding (spec '{spec}')"))?
            } else {
                let mut buf = [0u8; 4];
                ch.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
    };

    if mods.alt {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.push(0x1b);
        out.append(&mut bytes);
        return Ok(out);
    }
    Ok(bytes)
}

/// Legacy control-byte for `ctrl+<ch>`: maps the character into the C0 range
/// (`@A…Z[\]^_` and `?`).
fn control_byte(ch: char) -> Option<u8> {
    let upper = ch.to_ascii_uppercase();
    match upper {
        '@'..='_' => Some((upper as u8) & 0x1f),
        '?' => Some(0x7f),
        ' ' => Some(0x00),
        _ => None,
    }
}

/// kitty keyboard-protocol CSI-u form: `\e[<codepoint>;<mod>u`.
fn encode_kitty(base: &str, mods: Modifiers, spec: &str) -> Result<Vec<u8>, String> {
    // For escape-sequence keys, kitty accepts the same modified-CSI form as
    // xterm, so reuse the legacy encoder (it already threads the modifiers).
    if named_key(base).is_some() {
        return encode_legacy(base, mods, spec);
    }

    let codepoint: u32 = match base.to_lowercase().as_str() {
        "enter" | "return" | "cr" => 13,
        "tab" => 9,
        "esc" | "escape" => 27,
        "space" | "spc" => 32,
        "backspace" | "bs" => 127,
        _ => {
            let chars: Vec<char> = base.chars().collect();
            if chars.len() != 1 {
                return Err(format!("unknown key '{base}' in key spec '{spec}'"));
            }
            chars[0] as u32
        }
    };

    let code = mods.xterm_code();
    let seq = if code == 1 {
        format!("\x1b[{codepoint}u")
    } else {
        format!("\x1b[{codepoint};{code}u")
    };
    Ok(seq.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_letters() {
        assert_eq!(parse("ctrl+j").unwrap(), vec![0x0a]);
        assert_eq!(parse("ctrl+c").unwrap(), vec![0x03]);
        assert_eq!(parse("Ctrl+K").unwrap(), vec![0x0b]);
        assert_eq!(parse("ctrl+l").unwrap(), vec![0x0c]);
        assert_eq!(parse("ctrl+space").unwrap(), vec![0x00]);
    }

    #[test]
    fn alt_prepends_escape() {
        assert_eq!(parse("alt+enter").unwrap(), b"\x1b\r");
        assert_eq!(parse("alt+a").unwrap(), b"\x1ba");
        assert_eq!(parse("opt+/").unwrap(), b"\x1b/");
    }

    #[test]
    fn plain_named_keys() {
        assert_eq!(parse("enter").unwrap(), b"\r");
        assert_eq!(parse("tab").unwrap(), b"\t");
        assert_eq!(parse("esc").unwrap(), vec![0x1b]);
        assert_eq!(parse("space").unwrap(), b" ");
        assert_eq!(parse("backspace").unwrap(), vec![0x7f]);
    }

    #[test]
    fn shift_tab_is_backtab() {
        assert_eq!(parse("shift+tab").unwrap(), b"\x1b[Z");
    }

    #[test]
    fn shift_letter_uppercases() {
        assert_eq!(parse("shift+a").unwrap(), b"A");
    }

    #[test]
    fn arrows_and_modified_arrows() {
        assert_eq!(parse("up").unwrap(), b"\x1b[A");
        assert_eq!(parse("down").unwrap(), b"\x1b[B");
        assert_eq!(parse("ctrl+up").unwrap(), b"\x1b[1;5A");
        assert_eq!(parse("shift+left").unwrap(), b"\x1b[1;2D");
        assert_eq!(parse("alt+right").unwrap(), b"\x1b[1;3C");
    }

    #[test]
    fn function_keys() {
        assert_eq!(parse("f1").unwrap(), b"\x1bOP");
        assert_eq!(parse("f5").unwrap(), b"\x1b[15~");
        assert_eq!(parse("f12").unwrap(), b"\x1b[24~");
        assert_eq!(parse("delete").unwrap(), b"\x1b[3~");
        assert_eq!(parse("ctrl+delete").unwrap(), b"\x1b[3;5~");
    }

    #[test]
    fn kitty_shift_enter() {
        assert_eq!(parse("kitty:shift+enter").unwrap(), b"\x1b[13;2u");
        assert_eq!(parse("kitty:enter").unwrap(), b"\x1b[13u");
        assert_eq!(parse("kitty:ctrl+j").unwrap(), b"\x1b[106;5u");
    }

    #[test]
    fn plus_as_base_key() {
        // A literal `+` base key is written with a doubled separator.
        assert_eq!(parse("+").unwrap(), b"+");
        assert_eq!(parse("alt++").unwrap(), b"\x1b+");
        // '+' has no C0 control encoding, so `ctrl++` errors gracefully.
        assert!(parse("ctrl++").is_err());
    }

    #[test]
    fn errors() {
        assert!(parse("").is_err());
        assert!(parse("frobnicate").is_err());
        assert!(parse("ctrl+frob").is_err());
        assert!(parse("bogusmod+a").is_err());
    }
}
