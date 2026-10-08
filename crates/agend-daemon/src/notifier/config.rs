//! Parse non-secret configuration and resolve private token references.
use agend_core::config::{Config, SecretRef};
use std::{
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub fn parse(text: &str) -> Result<Config, String> {
    // Do not echo TOML parser diagnostics: malformed input may contain a token.
    let config: Config = toml::from_str(text).map_err(|_| "invalid config.toml".to_owned())?;
    if let Some(telegram) = &config.telegram {
        telegram.validate().map_err(str::to_owned)?;
    }
    Ok(config)
}

/// Encode references only; the operator CLI owns publishing this configuration.
pub fn encode(config: &Config) -> Result<String, String> {
    if let Some(telegram) = &config.telegram {
        telegram.validate()?;
    }
    toml::to_string(config).map_err(|_| "cannot encode Telegram configuration".into())
}

/// Missing configuration disables Telegram. Invalid present configuration fails boot.
pub fn load(home: &Path) -> Result<Option<(agend_core::config::TelegramConfig, Token)>, String> {
    let text = match std::fs::read_to_string(home.join("config.toml")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("cannot read config.toml".into()),
    };
    let Some(config) = parse(&text)?.telegram else {
        return Ok(None);
    };
    let token = resolve(&config.token)?;
    Ok(Some((config, token)))
}

/// Deliberately no Debug or Display, including on errors.
pub struct Token(String);
impl Token {
    pub(super) fn value(&self) -> &str {
        &self.0
    }
    pub fn parse(value: String) -> Result<Self, String> {
        let Some((id, secret)) = value.split_once(':') else {
            return Err("invalid Telegram bot token".into());
        };
        if id.is_empty()
            || id.len() > 20
            || !id.bytes().all(|c| c.is_ascii_digit())
            || secret.len() < 16
            || secret.len() > 128
            || !secret
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err("invalid Telegram bot token".into());
        }
        Ok(Self(value))
    }
}

pub fn resolve(reference: &SecretRef) -> Result<Token, String> {
    let value = match reference {
        SecretRef::Env(name) => std::env::var(name)
            .map_err(|_| "Telegram token environment variable is missing".to_owned())?,
        SecretRef::File(path) => {
            if !Path::new(path).is_absolute() {
                return Err("Telegram token file must be absolute".into());
            }
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|_| "cannot open Telegram token file".to_owned())?;
            let metadata = file
                .metadata()
                .map_err(|_| "cannot inspect Telegram token file".to_owned())?;
            if !metadata.is_file()
                || metadata.mode() & 0o077 != 0
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(
                    "Telegram token file must be an owned private regular file (0600)".into(),
                );
            }
            let mut value = String::new();
            file.take(257)
                .read_to_string(&mut value)
                .map_err(|_| "cannot read Telegram token file".to_owned())?;
            if value.len() > 256 {
                return Err("Telegram token file exceeds limit".into());
            }
            value.trim_end_matches(['\r', '\n']).to_owned()
        }
    };
    Token::parse(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn configuration_keeps_secrets_out_and_requires_explicit_sender_and_chat() {
        let text = "[telegram]\nchat_id = -100123456\nallow_user_ids = [42]\nneeds_you_topic = 7\ntoken = {kind = 'env', value = 'TELEGRAM_BOT_TOKEN'}\n";
        let config = parse(text).unwrap().telegram.unwrap();
        assert!(config.allows(-100123456, 42, false));
        assert!(!config.allows(42, 42, false));
        assert!(!config.allows(-100123456, 43, false));
        assert!(!config.allows(-100123456, 42, true));
        let mut empty = config.clone();
        empty.allow_user_ids.clear();
        assert!(!empty.allows(-100123456, 42, false));
        let secret = "123:DO_NOT_ECHO_THIS_SECRET";
        assert_eq!(
            parse(&text.replace(
                "{kind = 'env', value = 'TELEGRAM_BOT_TOKEN'}",
                &format!("'{secret}'")
            ))
            .unwrap_err(),
            "invalid config.toml"
        );
        assert!(parse(&text.replace("chat_id = -100123456", "chat_id = 0")).is_err());
        assert!(parse(&text.replace("kind = 'env'", "kind = 'inline'")).is_err());
    }
    #[test]
    fn token_file_refuses_public_symlink_oversized_and_non_regular_sources() {
        let dir = agend_testkit::tempdir::TempDir::new("telegram-secret").unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "123:abcdefghijklmnopqrstuvwxyz_123456789\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let reference = SecretRef::File(path.display().to_string());
        assert!(resolve(&reference).is_ok());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(resolve(&reference).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(resolve(&SecretRef::File(link.display().to_string())).is_err());
        std::fs::write(&path, "x".repeat(300)).unwrap();
        assert!(resolve(&reference).is_err());
        assert!(resolve(&SecretRef::File(dir.path().display().to_string())).is_err());
    }
}
