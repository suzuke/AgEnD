//! D40 P4: guard accidental GitHub merges, approvals and token disclosure.
//! This is a PATH seatbelt, not an authentication or network boundary.
//! Do not read gh config, request bodies, stdin, credentials or the DB.

use crate::audit::{self, Record};
use crate::ctx::{Ctx, MAX_DEPTH, lossy};
use crate::{Action, Outcome, Refusal};
use std::ffi::OsString;

pub fn plan(ctx: &Ctx, args: &[OsString]) -> Outcome {
    let argv = lossy(args);
    let parsed = Parsed::new(&argv);
    // Never put request payloads, headers or tokens into stderr or audit.
    let label: Vec<String> = match parsed.operands.as_slice() {
        ["pr", "merge" | "review", ..] | ["auth", "token", ..] => {
            parsed.operands[..2].iter().map(|s| s.to_string()).collect()
        }
        ["api", ..] => vec!["api".into()],
        _ => Vec::new(),
    };
    let record = |event: &str, code: Option<&str>| Record {
        ts: audit::now(),
        instance: ctx.instance.clone(),
        tool: "gh".into(),
        event: event.into(),
        code: code.map(String::from),
        argv: if parsed.operands.first() == Some(&"api") {
            vec!["api".into()]
        } else {
            label.clone()
        },
        cwd: ctx.cwd.clone().unwrap_or_default(),
        detail: None,
    };
    let verdict = if ctx.bypass {
        Ok(())
    } else if ctx.depth >= MAX_DEPTH {
        Err(Refusal::shim_loop("gh"))
    } else {
        parsed.check()
    };
    if let Err(refusal) = verdict {
        audit::append(ctx.home.as_deref(), &record("refuse", Some(refusal.code)));
        let command = if parsed.operands.first() == Some(&"api") {
            "gh api".into()
        } else {
            format!("gh {}", label.join(" ")).trim_end().to_string()
        };
        return Outcome {
            messages: refusal.render(&command),
            action: Action::Refuse(refusal),
        };
    }
    let Some(real) = ctx.find_real("gh") else {
        return Outcome::fail(
            "agend-shim: cannot find the real gh on PATH (the shim directory is excluded)",
        );
    };
    if ctx.bypass {
        audit::append(ctx.home.as_deref(), &record("bypass", None));
    }
    Outcome::exec(ctx.real_command(&real, args))
}

struct Parsed<'a> {
    operands: Vec<&'a str>,
    options: Vec<(&'a str, Option<&'a str>)>,
}

impl<'a> Parsed<'a> {
    fn new(args: &'a [String]) -> Self {
        let mut parsed = Self {
            operands: Vec::new(),
            options: Vec::new(),
        };
        let mut iter = args.iter().map(String::as_str);
        while let Some(arg) = iter.next() {
            if arg == "--" {
                parsed.operands.extend(iter);
                break;
            }
            if let Some(long) = arg.strip_prefix("--") {
                let (name, value) = long
                    .split_once('=')
                    .map_or((long, None), |(n, v)| (n, Some(v)));
                let value = if value.is_none() && takes_value(name) {
                    iter.next()
                } else {
                    value
                };
                parsed.options.push((name, value));
            } else if let Some(short) = arg.strip_prefix('-').filter(|s| !s.is_empty()) {
                for (offset, key) in short.char_indices() {
                    let name = &short[offset..offset + key.len_utf8()];
                    let rest = &short[offset + key.len_utf8()..];
                    if takes_value(name) {
                        let value = if rest.is_empty() {
                            iter.next()
                        } else {
                            Some(rest.trim_start_matches('='))
                        };
                        parsed.options.push((name, value));
                        break;
                    }
                    if let Some(value) = rest.strip_prefix('=') {
                        parsed.options.push((name, Some(value)));
                        break;
                    }
                    parsed.options.push((name, None));
                }
            } else {
                parsed.operands.push(arg);
            }
        }
        parsed
    }

    fn check(&self) -> Result<(), Refusal> {
        match self.operands.as_slice() {
            ["pr", "merge", ..] => Err(refuse("gh_merge", "merges go through the agend pipeline")),
            ["pr", "review", ..]
                if self.options.iter().any(|(name, value)| {
                    matches!(*name, "a" | "approve")
                        && !matches!(*value, Some("0" | "f" | "F" | "false" | "False" | "FALSE"))
                }) =>
            {
                Err(refuse(
                    "gh_approve",
                    "PR approvals go through the agend pipeline",
                ))
            }
            ["auth", "token", ..] => Err(refuse(
                "gh_token",
                "printing the GitHub authentication token is refused",
            )),
            ["api", endpoint, ..] => self.check_api(endpoint),
            _ => Ok(()),
        }
    }

    fn check_api(&self, endpoint: &str) -> Result<(), Refusal> {
        let path = endpoint.split(['?', '#']).next().unwrap_or_default();
        let path = path
            .split_once("://")
            .map_or(path, |(_, url)| url.split_once('/').map_or("", |(_, p)| p));
        let path = decode_path(path);
        let segments: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for (index, segment) in segments.iter().enumerate() {
            if *segment != "repos" {
                continue;
            }
            let tail = segments.get(index + 3..).unwrap_or_default();
            if matches!(tail, ["merges", ..] | ["pulls", _, "merge" | "reviews", ..]) {
                return Err(refuse(
                    "gh_api_merge_review",
                    "merge and review API endpoints are reserved for the agend pipeline",
                ));
            }
        }
        if segments.last() != Some(&"graphql") {
            return Ok(());
        }
        for (name, value) in &self.options {
            if *name == "input" {
                return Err(refuse(
                    "gh_graphql_body",
                    "GraphQL request bodies from files or stdin cannot be checked before execution",
                ));
            }
            if !matches!(*name, "f" | "F" | "field" | "raw-field") {
                continue;
            }
            let Some(query) = value.and_then(|v| v.strip_prefix("query=")) else {
                continue;
            };
            if *name != "f" && *name != "raw-field" && query.starts_with('@') {
                return Err(refuse(
                    "gh_graphql_body",
                    "GraphQL queries from files or stdin cannot be checked before execution",
                ));
            }
            if guarded_graphql(query) {
                return Err(refuse(
                    "gh_api_merge_review",
                    "GraphQL merge and review mutations are reserved for the agend pipeline",
                ));
            }
        }
        Ok(())
    }
}

fn takes_value(name: &str) -> bool {
    matches!(
        name,
        "R" | "repo"
            | "b"
            | "body"
            | "body-file"
            | "F"
            | "f"
            | "field"
            | "raw-field"
            | "H"
            | "header"
            | "hostname"
            | "X"
            | "method"
            | "p"
            | "preview"
            | "q"
            | "jq"
            | "t"
            | "template"
            | "cache"
            | "input"
    )
}

fn refuse(code: &'static str, reason: &str) -> Refusal {
    Refusal::new(
        code,
        reason,
        "report your result with `agend done` or `agend review`; ask the operator if GitHub access needs attention",
    )
}

fn decode_path(path: &str) -> String {
    let mut bytes = path.bytes();
    let mut out = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let mut peek = bytes.clone();
            let code = peek
                .next()
                .and_then(|a| char::from(a).to_digit(16))
                .zip(peek.next().and_then(|b| char::from(b).to_digit(16)));
            if let Some((a, b)) = code {
                out.push((a * 16 + b) as u8);
                bytes = peek;
                continue;
            }
        }
        out.push(byte);
    }
    String::from_utf8_lossy(&out).to_ascii_lowercase()
}

// Recognize identifiers, skipping comments and quoted strings. This is not
// a GraphQL validator; aliases do not hide the underlying mutation name.
fn guarded_graphql(query: &str) -> bool {
    let mut chars = query.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '#' {
            for ch in chars.by_ref() {
                if ch == '\n' {
                    break;
                }
            }
        } else if ch == '"' {
            while let Some(ch) = chars.next() {
                if ch == '\\' {
                    chars.next();
                } else if ch == '"' {
                    break;
                }
            }
        } else if ch == '_' || ch.is_ascii_alphabetic() {
            let mut name = String::from(ch);
            while chars
                .peek()
                .is_some_and(|c| *c == '_' || c.is_ascii_alphanumeric())
            {
                name.push(chars.next().unwrap());
            }
            if matches!(
                name.as_str(),
                "mergePullRequest"
                    | "enablePullRequestAutoMerge"
                    | "enqueuePullRequest"
                    | "mergeBranch"
                    | "addPullRequestReview"
                    | "submitPullRequestReview"
            ) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests;
