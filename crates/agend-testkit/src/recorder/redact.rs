//! Redaction of recorded messages, and the secret scan that must come back
//! empty before a transcript is written.
//!
//! Redaction keeps every value's JSON type (so shape comparison still works)
//! and maps each distinct id to a stable placeholder (`<uuid-1>`, `<ses-2>`,
//! …) so a transcript still shows which messages refer to the same thing.
//!
//! | What | Becomes |
//! |---|---|
//! | the scenario directory (also as `/tmp/…` and as claude's `-private-tmp-…` slug) | `<rec>`, `<rec-slug>` |
//! | `$TMPDIR`, `$HOME`, the user name, the host name | `<tmpdir>`, `~`, `<user>`, `<host>` |
//! | UUIDs anywhere in a string | `<uuid-N>` |
//! | prefixed ids (`ses_…`, `msg_…`, `toolu_…`, `call_…`) | `<ses-N>`, `<msg-N>`, … |
//! | hex runs of 24+ chars, other 40+ char tokens, JWTs | `<hex-N>`, `<token-N>`, `<jwt>` |
//! | e-mail addresses, `sk-…` keys | `<email>`, `<sk-key>` |
//! | wall-clock times (`2026-09-25T14-12-36` in codex rollout file names, `…T06:15:18.454Z` in opencode titles): local time next to UTC would give away the time zone | `<time>` |
//! | the uid in `/tmp/<name>-<uid>` (`/private/tmp/claude-501/…`) | `/tmp/<name>-<uid>` |
//! | values under secret-looking keys (`token`, `apiKey`, `authorization`, `email`, `accountId`, …), account and machine details (`planType`, `userAgent`, usage limits, `platformOs`, `authMode`) | `"<redacted>"`, `0`, or the same for every leaf |
//! | the user's own MCP servers (codex `mcpServer/startupStatus/updated`, one per server and state change) | ONE entry, every leaf blanked; the others are dropped, so neither names nor count remain |
//!
//! Object keys are redacted like strings (maps keyed by session id).
//!
//! [`scan`] also flags every word of a local denylist (e.g. the names of
//! the user's MCP servers), read from `AGEND_RECORD_DENYLIST` (words
//! separated by commas or whitespace) and from the file
//! `AGEND_RECORD_DENYLIST_FILE` (default `~/.config/agend-record/denylist`,
//! one word per line, `#` comments). Never commit that list.
//!
//! Must NOT: change a value's JSON type.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

use super::Entry;

/// Keys (lowercase, without `_` and `-`) whose values are always redacted.
const SECRET_KEYS: &[&str] = &[
    "token",
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "authtoken",
    "sessiontoken",
    "apikey",
    "authorization",
    "cookie",
    "setcookie",
    "password",
    "secret",
    "clientsecret",
    "credentials",
    "email",
    "accountid",
    "chatgptaccountid",
    "userid",
    "orgid",
    "organizationid",
    "machineid",
    "deviceid",
    // Account and machine details (codex `account/*`, `initialize`).
    "plantype",
    "useragent",
    "usedpercent",
    "resetsat",
    "balance",
    "windowdurationmins",
    "platformos",
    "authmode",
];

/// Methods whose messages describe the user's machine, one per item (codex
/// sends one `mcpServer/startupStatus/updated` per MCP server and state):
/// only the first is kept, blanked, so the number of items is not recorded.
const ONE_BLANK_ENTRY: &[&str] = &["mcpServer/startupStatus/updated"];

pub struct Redactor {
    /// (literal, placeholder), longest literal first.
    literals: Vec<(String, String)>,
    ids: RefCell<BTreeMap<String, String>>,
    counters: RefCell<BTreeMap<String, usize>>,
}

impl Redactor {
    /// A redactor for a scenario that ran in `dir`.
    pub fn new(dir: &Path) -> Redactor {
        let mut literals = Vec::new();
        let dir = dir.to_string_lossy().trim_end_matches('/').to_owned();
        for form in [dir.clone(), dir.trim_start_matches("/private").to_owned()] {
            if form.len() > 1 {
                literals.push((slug(&form), "<rec-slug>".to_owned()));
                literals.push((form, "<rec>".to_owned()));
            }
        }
        let tmp = std::env::temp_dir();
        let tmp = tmp.to_string_lossy().trim_end_matches('/').to_owned();
        for form in [format!("/private{tmp}"), tmp] {
            if form.len() > 8 {
                literals.push((form, "<tmpdir>".to_owned()));
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = home.to_string_lossy().trim_end_matches('/').to_owned();
            if home.len() > 1 {
                literals.push((slug(&home), "-<home-slug>".to_owned()));
                literals.push((home, "~".to_owned()));
            }
        }
        for (name, placeholder) in [(user_name(), "<user>"), (host_name(), "<host>")] {
            if name.len() >= 3 {
                literals.push((name, placeholder.to_owned()));
            }
        }
        literals.sort_by_key(|(literal, _)| std::cmp::Reverse(literal.len()));
        Redactor {
            literals,
            ids: RefCell::default(),
            counters: RefCell::default(),
        }
    }

    pub fn entries(&self, entries: &[Entry]) -> Vec<Entry> {
        let mut seen = Vec::new();
        entries
            .iter()
            .filter_map(|e| {
                let mut msg = e.msg.clone();
                if let Some(method) = ONE_BLANK_ENTRY.iter().find(|m| msg["method"] == **m) {
                    if seen.contains(method) {
                        return None;
                    }
                    seen.push(method);
                    if let Some(params) = msg.get_mut("params") {
                        *params = blank(params);
                    }
                }
                Some(Entry {
                    from: e.from,
                    via: e.via.clone(),
                    msg: self.value(&msg),
                })
            })
            .collect()
    }

    pub fn value(&self, value: &Value) -> Value {
        self.walk(value)
    }

    fn walk(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.string(s)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.walk(v)).collect()),
            Value::Object(map) => {
                let mut out = Map::new();
                for (key, v) in map {
                    let v = if is_secret_key(key) {
                        blank(v)
                    } else {
                        self.walk(v)
                    };
                    out.insert(self.string(key), v);
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }

    pub fn string(&self, s: &str) -> String {
        let mut s = s.to_owned();
        for (literal, placeholder) in &self.literals {
            if s.contains(literal.as_str()) {
                s = s.replace(literal.as_str(), placeholder);
            }
        }
        let s = self.uuids(&s);
        let s = emails(&s);
        let s = times(&s);
        let s = tmp_uids(&s);
        self.words(&s)
    }

    fn placeholder(&self, kind: &str, raw: &str) -> String {
        let mut ids = self.ids.borrow_mut();
        if let Some(p) = ids.get(raw) {
            return p.clone();
        }
        let mut counters = self.counters.borrow_mut();
        let n = counters.entry(kind.to_owned()).or_insert(0);
        *n += 1;
        let p = format!("<{kind}-{n}>");
        ids.insert(raw.to_owned(), p.clone());
        p
    }

    fn uuids(&self, s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < bytes.len() {
            if i + 36 <= bytes.len() && is_uuid(&bytes[i..i + 36]) {
                out.push_str(&self.placeholder("uuid", &s[i..i + 36]));
                i += 36;
            } else {
                let ch = s[i..].chars().next().unwrap_or_default();
                out.push(ch);
                i += ch.len_utf8().max(1);
            }
        }
        out
    }

    /// Redacts id-like words (`[A-Za-z0-9_-]+` runs).
    fn words(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut word = String::new();
        for ch in s.chars().chain(std::iter::once('\u{0}')) {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                word.push(ch);
                continue;
            }
            if !word.is_empty() {
                out.push_str(&self.word(&word));
                word.clear();
            }
            if ch != '\u{0}' {
                out.push(ch);
            }
        }
        out
    }

    fn word(&self, w: &str) -> String {
        if w.starts_with("eyJ") && w.len() >= 16 {
            return "<jwt>".to_owned();
        }
        if is_sk_key(w) {
            return "<sk-key>".to_owned();
        }
        // `ses_2e9c…`, `toolu_01…`, `call_function_ehm47i2fsh0p_1`; a
        // snake_case word (`stop_hook_active`) has no digit after the prefix.
        if let Some((prefix, rest)) = w.split_once('_')
            && (2..=8).contains(&prefix.len())
            && prefix.chars().all(|c| c.is_ascii_alphabetic())
            && rest.len() >= 10
            && (rest.chars().all(|c| c.is_ascii_alphanumeric())
                || (rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && rest.chars().any(|c| c.is_ascii_digit())))
        {
            return self.placeholder(&prefix.to_ascii_lowercase(), w);
        }
        if w.len() >= 24 && w.chars().all(|c| c.is_ascii_hexdigit()) {
            return self.placeholder("hex", w);
        }
        let digits = w.chars().any(|c| c.is_ascii_digit());
        let letters = w.chars().any(|c| c.is_ascii_alphabetic());
        if w.len() >= 40 && digits && letters && !w.contains("--") {
            return self.placeholder("token", w);
        }
        w.to_owned()
    }
}

/// `sk-…` API keys (`sk-ant-…`, `sk-proj-…`), also short ones.
fn is_sk_key(word: &str) -> bool {
    word.strip_prefix("sk-").is_some_and(|rest| {
        rest.len() >= 12
            && rest.chars().any(|c| c.is_ascii_digit())
            && rest.chars().any(|c| c.is_ascii_alphabetic())
    })
}

/// Replaces ISO-8601 date-times (`2026-09-25T14-12-36`,
/// `2026-09-25T06:15:18.454Z`) with `<time>`; plain dates stay.
fn times(s: &str) -> String {
    let b = s.as_bytes();
    let digits =
        |i: usize, n: usize| i + n <= b.len() && b[i..i + n].iter().all(u8::is_ascii_digit);
    let at = |i: usize, set: &[u8]| i < b.len() && set.contains(&b[i]);
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        let hit = digits(i, 4)
            && at(i + 4, b"-")
            && digits(i + 5, 2)
            && at(i + 7, b"-")
            && digits(i + 8, 2)
            && at(i + 10, b"T")
            && digits(i + 11, 2)
            && at(i + 13, b":-")
            && digits(i + 14, 2)
            && at(i + 16, b":-")
            && digits(i + 17, 2);
        if !hit {
            i += 1;
            continue;
        }
        let mut end = i + 19;
        if at(end, b".") && digits(end + 1, 1) {
            end += 1;
            while digits(end, 1) {
                end += 1;
            }
        }
        if at(end, b"Z") {
            end += 1;
        }
        out.push_str(&s[copied..i]);
        out.push_str("<time>");
        copied = end;
        i = end;
    }
    out.push_str(&s[copied..]);
    out
}

/// `/tmp/<name>-<digits>` (a per-uid directory such as
/// `/private/tmp/claude-501`) → `/tmp/<name>-<uid>`.
fn tmp_uids(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/tmp/") {
        let (head, tail) = rest.split_at(i + 5);
        out.push_str(head);
        let seg_len = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .unwrap_or(tail.len());
        let seg = &tail[..seg_len];
        match seg.rsplit_once('-') {
            Some((name, uid))
                if !name.is_empty()
                    && !uid.is_empty()
                    && uid.chars().all(|c| c.is_ascii_digit()) =>
            {
                out.push_str(name);
                out.push_str("-<uid>");
            }
            _ => out.push_str(seg),
        }
        rest = &tail[seg_len..];
    }
    out.push_str(rest);
    out
}

fn is_secret_key(key: &str) -> bool {
    let k: String = key
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    SECRET_KEYS.contains(&k.as_str())
        || k.contains("secret")
        || k.contains("password")
        || k.contains("apikey")
}

/// The same shape with every leaf blanked.
fn blank(value: &Value) -> Value {
    match value {
        Value::String(_) => Value::String("<redacted>".to_owned()),
        Value::Number(_) => Value::from(0),
        Value::Array(items) => Value::Array(items.iter().map(blank).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), blank(v))).collect())
        }
        other => other.clone(),
    }
}

fn is_uuid(b: &[u8]) -> bool {
    b.iter().enumerate().all(|(i, c)| match i {
        8 | 13 | 18 | 23 => *c == b'-',
        _ => c.is_ascii_hexdigit(),
    })
}

fn email_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')
}

/// Replaces `local@domain.tld` with `<email>`.
fn emails(s: &str) -> String {
    if !s.contains('@') {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut start = i;
            while start > 0 && email_char(chars[start - 1]) {
                start -= 1;
            }
            let mut end = i + 1;
            while end < chars.len() && email_char(chars[end]) {
                end += 1;
            }
            let domain: String = chars[i + 1..end].iter().collect();
            if start < i && domain.contains('.') && !domain.starts_with('.') {
                let keep = out.chars().count() - (i - start);
                out = out.chars().take(keep).collect();
                out.push_str("<email>");
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Claude Code's project directory name for a path: every character that is
/// not ASCII alphanumeric becomes `-`.
pub fn slug(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
}

fn host_name() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// Everything in a redacted transcript that still looks secret or
/// user-specific. Must be empty before the transcript is written. Uses the
/// local denylist (see the module docs).
pub fn scan(header: &Value, entries: &[Entry]) -> Vec<String> {
    scan_with(header, entries, &denylist())
}

/// [`scan`] with an explicit denylist: words (case-insensitive, whole word)
/// that must not appear anywhere.
pub fn scan_with(header: &Value, entries: &[Entry], deny: &[String]) -> Vec<String> {
    let mut text = header.to_string();
    for entry in entries {
        text.push('\n');
        text.push_str(&entry.to_json().to_string());
    }
    let mut findings = Vec::new();
    let mut literal = |needle: &str, what: &str| {
        if needle.len() >= 3 && text.contains(needle) {
            findings.push(what.to_owned());
        }
    };
    literal("/Users/", "home path /Users/");
    literal("/home/", "home path /home/");
    literal("/var/folders/", "TMPDIR path");
    literal("/private/tmp/agend-rec-", "scenario dir");
    literal("-private-tmp-agend-rec-", "scenario dir slug");
    literal("eyJ", "JWT-like text");
    literal("Bearer ", "bearer token");
    literal(&user_name(), "user name");
    literal(&host_name(), "host name");
    for secret in known_secrets() {
        literal(&secret, "a value from a CLI auth file");
    }
    if emails(&text) != text {
        findings.push("e-mail address".to_owned());
    }
    let bytes = text.as_bytes();
    if (0..bytes.len().saturating_sub(35)).any(|i| is_uuid(&bytes[i..i + 36])) {
        findings.push("UUID".to_owned());
    }
    let probe = Redactor {
        literals: Vec::new(),
        ids: RefCell::default(),
        counters: RefCell::default(),
    };
    if probe.words(&text) != text {
        findings.push("id or token-like word".to_owned());
    }
    if times(&text) != text {
        findings.push("wall-clock time (gives away the time zone)".to_owned());
    }
    if tmp_uids(&text) != text {
        findings.push("uid in a /tmp path".to_owned());
    }
    let mut blanked = 0;
    let mut values = vec![header];
    values.extend(entries.iter().map(|e| &e.msg));
    for value in values {
        secret_leaves(value, &mut findings);
        if ONE_BLANK_ENTRY.iter().any(|m| value["method"] == *m) {
            blanked += 1;
            if value["params"] != blank(&value["params"]) {
                findings.push(format!("{} not blanked", value["method"]));
            }
        }
    }
    if blanked > 1 {
        findings.push(format!("{blanked} machine-list entries (at most 1)"));
    }
    let words: Vec<String> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map(str::to_ascii_lowercase)
        .collect();
    for (n, word) in deny.iter().enumerate() {
        let word = word.to_ascii_lowercase();
        let found = if word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            words.contains(&word)
        } else {
            text.to_ascii_lowercase().contains(&word)
        };
        if found {
            // The word itself is not printed: it is private.
            findings.push(format!("denylist word #{}", n + 1));
        }
    }
    for i in text.match_indices("sk-").map(|(i, _)| i) {
        let before = text[..i].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        let word: String = text[i..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if is_sk_key(&word) {
            findings.push("sk- key".to_owned());
            break;
        }
    }
    findings
}

/// Values under secret keys that are not blanked.
fn secret_leaves(value: &Value, findings: &mut Vec<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|v| secret_leaves(v, findings)),
        Value::Object(map) => {
            for (k, v) in map {
                if is_secret_key(k) && *v != blank(v) {
                    findings.push(format!("value under `{k}`"));
                } else {
                    secret_leaves(v, findings);
                }
            }
        }
        _ => {}
    }
}

/// The local denylist: `AGEND_RECORD_DENYLIST` plus the file
/// `AGEND_RECORD_DENYLIST_FILE` (default `~/.config/agend-record/denylist`).
pub fn denylist() -> Vec<String> {
    let mut out: Vec<String> = std::env::var("AGEND_RECORD_DENYLIST")
        .unwrap_or_default()
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    let file = std::env::var_os("AGEND_RECORD_DENYLIST_FILE")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| Path::new(&h).join(".config/agend-record/denylist"))
        });
    if let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) {
        out.extend(
            text.lines()
                .map(|l| l.split('#').next().unwrap_or_default().trim())
                .filter(|w| !w.is_empty())
                .map(str::to_owned),
        );
    }
    out
}

/// String values (16+ chars) from the CLIs' auth files, read only. None of
/// them may appear in a transcript.
fn known_secrets() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let home = Path::new(&home);
    let mut out = Vec::new();
    for file in [".codex/auth.json", ".local/share/opencode/auth.json"] {
        let Ok(text) = std::fs::read_to_string(home.join(file)) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            collect_strings(&value, &mut out);
        }
    }
    out
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) if s.len() >= 16 => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_become_stable_placeholders_and_types_are_kept() {
        let r = Redactor::new(Path::new("/private/tmp/agend-rec-x-AbCd"));
        let v = json!({
            "threadId": "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b",
            "again": "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b",
            "session": "ses_2e9c3b0a1ffeLmLgCA56",
            "call": "call_function_ehm47i2fsh0p_1",
            "snake": "cache_creation_input_tokens",
            "cwd": "/private/tmp/agend-rec-x-AbCd/project",
            "slug": "-private-tmp-agend-rec-x-AbCd-project",
            "mail": "write to someone@example.com now",
            "accessToken": "abc",
            "accountId": 42,
            "input_tokens": 7,
            "stop_hook_active": true
        });
        let out = r.value(&v);
        assert_eq!(out["threadId"], "<uuid-1>");
        assert_eq!(out["again"], "<uuid-1>");
        assert_eq!(out["session"], "<ses-1>");
        assert_eq!(out["call"], "<call-1>");
        assert_eq!(out["snake"], "cache_creation_input_tokens");
        assert_eq!(out["cwd"], "<rec>/project");
        assert_eq!(out["slug"], "<rec-slug>-project");
        assert_eq!(out["mail"], "write to <email> now");
        assert_eq!(out["accessToken"], "<redacted>");
        assert_eq!(out["accountId"], 0);
        assert_eq!(out["input_tokens"], 7);
        assert_eq!(out["stop_hook_active"], true);
    }

    #[test]
    fn scan_flags_what_redaction_would_have_caught() {
        let entry = |msg: Value| Entry {
            from: super::super::Side::Backend,
            via: "ws".into(),
            msg,
        };
        let header = json!({"type": "header"});
        assert!(scan(&header, &[entry(json!({"id": "<uuid-1>", "text": "OK"}))]).is_empty());
        let leaked = [
            json!({"id": "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"}),
            json!({"p": "/Users/someone/x"}),
            json!({"m": "a@b.co"}),
            json!({"s": "ses_2e9c3b0a1ffeLmLgCA56"}),
        ];
        for msg in leaked {
            assert!(!scan(&header, &[entry(msg.clone())]).is_empty(), "{msg}");
        }
    }

    #[test]
    fn machine_details_times_uids_and_mcp_servers_are_redacted() {
        let r = Redactor::new(Path::new("/private/tmp/agend-rec-x-AbCd"));
        let v = json!({
            "path": "~/.codex/sessions/2026/09/25/rollout-2026-09-25T14-12-36-x.jsonl",
            "title": "New session - 2026-09-25T06:15:18.454Z",
            "protocolVersion": "2025-11-25",
            "scratchpad_dir": "/private/tmp/claude-501/p/scratchpad",
            "platformOs": "macos",
            "authMode": "chatgpt",
            "key": "sk-proj-abc123def456ghi789",
        });
        let out = r.value(&v);
        assert_eq!(
            out["path"],
            "~/.codex/sessions/2026/09/25/rollout-<time>-x.jsonl"
        );
        assert_eq!(out["title"], "New session - <time>");
        assert_eq!(out["protocolVersion"], "2025-11-25");
        assert_eq!(
            out["scratchpad_dir"],
            "/private/tmp/claude-<uid>/p/scratchpad"
        );
        assert_eq!(out["platformOs"], "<redacted>");
        assert_eq!(out["authMode"], "<redacted>");
        assert_eq!(out["key"], "<sk-key>");

        let status = |name: &str, state: &str| Entry {
            from: super::super::Side::Backend,
            via: "ws".into(),
            msg: json!({"method": "mcpServer/startupStatus/updated",
                "params": {"name": name, "status": state, "error": null}}),
        };
        let entries = [
            status("a", "starting"),
            status("b", "starting"),
            status("a", "ready"),
        ];
        let out = r.entries(&entries);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].msg["params"],
            json!({"name": "<redacted>", "status": "<redacted>", "error": null})
        );
        assert_eq!(r.entries(&out), out, "idempotent");
        assert!(scan_with(&json!({}), &out, &[]).is_empty());
    }

    #[test]
    fn scan_flags_short_keys_times_uids_machine_details_and_denylisted_words() {
        let entry = |msg: Value| Entry {
            from: super::super::Side::Backend,
            via: "ws".into(),
            msg,
        };
        let header = json!({"type": "header"});
        let deny = vec!["myserver".to_owned()];
        let clean = [entry(
            json!({"text": "ask-me <sk-key> myservers task-12345678901234"}),
        )];
        assert_eq!(scan_with(&header, &clean, &deny), Vec::<String>::new());
        let leaked = [
            json!({"k": "key sk-proj-abc123def456ghi789"}),
            json!({"t": "New session - 2026-09-25T06:15:18.454Z"}),
            json!({"p": "/private/tmp/claude-501/x"}),
            json!({"platformOs": "macos"}),
            json!({"name": "MyServer"}),
        ];
        for msg in leaked {
            let found = scan_with(&header, &[entry(msg.clone())], &deny);
            assert!(!found.is_empty(), "{msg}");
            assert!(!found.iter().any(|f| f.contains("yserver")), "{found:?}");
        }
        let twice = [
            entry(
                json!({"method": "mcpServer/startupStatus/updated", "params": {"name": "<redacted>"}}),
            ),
            entry(
                json!({"method": "mcpServer/startupStatus/updated", "params": {"name": "<redacted>"}}),
            ),
        ];
        assert!(!scan_with(&header, &twice, &[]).is_empty());
    }
}
