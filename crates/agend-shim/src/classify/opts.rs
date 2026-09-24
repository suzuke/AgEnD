//! Exact-spelling option parser for the git subcommands the shim inspects
//! (deny-by-default).
//!
//! git's parse-options accepts any unambiguous prefix of a long option
//! (`--mirr` = `--mirror`) and attached short values (`-bfoo` = `-b foo`).
//! A checker that looks for exact strings therefore misses them. This parser
//! mirrors git's rules (option clusters, attached and separate values, `--`,
//! `--end-of-options`, options after positionals) but only accepts options
//! spelled exactly as listed in the command's spec; anything else is an
//! error the caller refuses.
//!
//! Spec entries: `"-f|--force"` (flag), `"-o|--push-option="` (required
//! value), `"--force-with-lease?"` (optional value, `--x=v` / `-xv` only),
//! `"--contains*"` (takes the next argument when there is one, like git's
//! `PARSE_OPT_LASTARG_DEFAULT`). The canonical name of an option is its long
//! form, or the short form when it has none.
//!
//! Must NOT: accept an abbreviated or unlisted option.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arity {
    Flag,
    Required,
    Optional,
    LastArgDefault,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    short: Option<char>,
    long: Option<&'static str>,
    arity: Arity,
    canonical: &'static str,
}

fn entry(spec: &'static str) -> Entry {
    let (body, arity) = match spec.as_bytes().last() {
        Some(b'=') => (&spec[..spec.len() - 1], Arity::Required),
        Some(b'?') => (&spec[..spec.len() - 1], Arity::Optional),
        Some(b'*') => (&spec[..spec.len() - 1], Arity::LastArgDefault),
        _ => (spec, Arity::Flag),
    };
    let mut e = Entry {
        short: None,
        long: None,
        arity,
        canonical: body,
    };
    for part in body.split('|') {
        if let Some(l) = part.strip_prefix("--") {
            e.long = Some(l);
            e.canonical = part;
        } else if let Some(s) = part.strip_prefix('-') {
            e.short = s.chars().next();
            if e.long.is_none() {
                e.canonical = part;
            }
        }
    }
    e
}

/// A parsed argv of one subcommand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Options in order: canonical name and value.
    pub opts: Vec<(&'static str, Option<String>)>,
    /// Positionals before `--`.
    pub pos: Vec<String>,
    /// Arguments after `--`.
    pub paths: Vec<String>,
    /// Whether `--` was given.
    pub dashdash: bool,
}

impl Parsed {
    pub fn has(&self, name: &str) -> bool {
        self.opts.iter().any(|(n, _)| *n == name)
    }

    pub fn any(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.has(n))
    }

    /// The first of `names` present, for messages.
    pub fn first_of(&self, names: &[&'static str]) -> Option<&'static str> {
        self.opts
            .iter()
            .map(|(n, _)| *n)
            .find(|n| names.contains(n))
    }

    /// Values given to option `name`, in order.
    pub fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.opts
            .iter()
            .filter(move |(n, _)| *n == name)
            .filter_map(|(_, v)| v.as_deref())
    }

    /// The last value of option `name` (git: last one wins).
    pub fn value(&self, name: &str) -> Option<&str> {
        self.opts
            .iter()
            .rev()
            .find(|(n, v)| *n == name && v.is_some())
            .and_then(|(_, v)| v.as_deref())
    }
}

/// An option the spec does not list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unknown {
    pub arg: String,
    /// The listed long option this looks like an abbreviation of.
    pub like: Option<&'static str>,
}

pub fn parse(spec: &[&[&'static str]], args: &[String]) -> Result<Parsed, Unknown> {
    let entries: Vec<Entry> = spec
        .iter()
        .flat_map(|g| g.iter())
        .map(|s| entry(s))
        .collect();
    let mut out = Parsed::default();
    let mut i = 0;
    let mut only_positionals = false;
    while i < args.len() {
        let a = args[i].as_str();
        i += 1;
        if out.dashdash {
            out.paths.push(a.to_string());
            continue;
        }
        if only_positionals || a == "-" || !a.starts_with('-') {
            out.pos.push(a.to_string());
            continue;
        }
        if a == "--" {
            out.dashdash = true;
            continue;
        }
        if a == "--end-of-options" {
            only_positionals = true;
            continue;
        }
        let unknown = |like| Unknown {
            arg: a.to_string(),
            like,
        };
        if let Some(long) = a.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let Some(e) = entries.iter().find(|e| e.long == Some(name)) else {
                let like = entries
                    .iter()
                    .filter_map(|e| e.long)
                    .find(|l| !name.is_empty() && l.starts_with(name));
                return Err(unknown(like));
            };
            let value = match (e.arity, attached) {
                (Arity::Flag, Some(_)) => return Err(unknown(None)),
                (Arity::Flag, None) => None,
                (Arity::Optional, v) => v,
                (_, Some(v)) => Some(v),
                (_, None) => take_next(args, &mut i),
            };
            out.opts.push((e.canonical, value));
            continue;
        }
        // A cluster of short options: `-fd`, `-bname`, `-n5`.
        let cluster = &a[1..];
        for (at, c) in cluster.char_indices() {
            let Some(e) = entries.iter().find(|e| e.short == Some(c)) else {
                return Err(unknown(None));
            };
            let rest = &cluster[at + c.len_utf8()..];
            let value = match e.arity {
                Arity::Flag => {
                    out.opts.push((e.canonical, None));
                    continue;
                }
                Arity::Optional => (!rest.is_empty()).then(|| rest.to_string()),
                _ if !rest.is_empty() => Some(rest.to_string()),
                _ => take_next(args, &mut i),
            };
            out.opts.push((e.canonical, value));
            break;
        }
    }
    Ok(out)
}

fn take_next(args: &[String], i: &mut usize) -> Option<String> {
    let v = args.get(*i).cloned();
    if v.is_some() {
        *i += 1;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &[&str] = &[
        "-f|--force",
        "-d|--delete",
        "-D",
        "-b=",
        "-o|--push-option=",
        "--force-with-lease?",
        "-n?",
        "--contains*",
        "--mirror",
    ];

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn p(s: &str) -> Result<Parsed, Unknown> {
        parse(&[SPEC], &argv(s))
    }

    #[test]
    fn exact_spellings_parse_like_git() {
        let r = p("-fd origin -o x --push-option=y HEAD:a --force-with-lease=a:b -- p").unwrap();
        assert!(r.has("--force") && r.has("--delete"));
        assert_eq!(r.values("--push-option").collect::<Vec<_>>(), ["x", "y"]);
        assert_eq!(r.value("--force-with-lease"), Some("a:b"));
        assert_eq!(r.pos, argv("origin HEAD:a"));
        assert_eq!(r.paths, ["p"]);
        assert!(r.dashdash);
        let r = p("-bfoo").unwrap();
        assert_eq!(r.value("-b"), Some("foo"));
        let r = p("-b foo x").unwrap();
        assert_eq!(
            (r.value("-b"), r.pos.as_slice()),
            (Some("foo"), &argv("x")[..])
        );
        let r = p("-n5 -D").unwrap();
        assert_eq!(r.value("-n"), Some("5"));
        assert!(r.has("-D"));
        assert_eq!(p("-n x").unwrap().pos, ["x"], "optional values attach only");
        assert_eq!(
            p("--contains abc").unwrap().value("--contains"),
            Some("abc")
        );
        assert_eq!(p("--contains").unwrap().value("--contains"), None);
        let r = p("--end-of-options --force -").unwrap();
        assert_eq!(r.pos, ["--force", "-"]);
        assert!(r.opts.is_empty());
    }

    #[test]
    fn abbreviations_and_unlisted_options_are_errors() {
        assert_eq!(
            p("--mirr origin").unwrap_err(),
            Unknown {
                arg: "--mirr".into(),
                like: Some("mirror")
            }
        );
        for bad in ["--forc", "--all", "-x", "-fx", "--force=yes", "--no-force"] {
            assert!(p(bad).is_err(), "{bad}");
        }
    }
}
