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
//! | e-mail addresses | `<email>` |
//! | values under secret-looking keys (`token`, `apiKey`, `authorization`, `email`, `accountId`, …) and account details (`planType`, `userAgent`, usage limits) | `"<redacted>"`, `0`, or the same for every leaf |
//! | the names of the user's MCP servers (codex `mcpServer/startupStatus/updated`) | `"<redacted>"` |
//!
//! Object keys are redacted like strings (maps keyed by session id).
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
];

/// `(method, field in params)`: user-specific values inside one message
/// kind (the names of the user's own MCP servers).
const SECRET_PARAMS: &[(&str, &str)] = &[("mcpServer/startupStatus/updated", "name")];

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
        entries
            .iter()
            .map(|e| Entry {
                from: e.from,
                via: e.via.clone(),
                msg: self.value(&e.msg),
            })
            .collect()
    }

    pub fn value(&self, value: &Value) -> Value {
        let mut value = value.clone();
        for (method, field) in SECRET_PARAMS {
            if value["method"] == *method
                && let Some(v) = value.get_mut("params").and_then(|p| p.get_mut(*field))
            {
                *v = blank(v);
            }
        }
        self.walk(&value)
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
/// user-specific. Must be empty before the transcript is written.
pub fn scan(header: &Value, entries: &[Entry]) -> Vec<String> {
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
    if let Some(i) = text.find("sk-")
        && text[i + 3..]
            .chars()
            .take(10)
            .filter(char::is_ascii_alphanumeric)
            .count()
            == 10
    {
        findings.push("sk- key".to_owned());
    }
    findings
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
}
