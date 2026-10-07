//! User-service definitions. No shell, filesystem or service-manager calls.
//! Installation ownership and publication belong to the executable's I/O layer.

use alloc::{format, string::String};
use serde::{Deserialize, Serialize};

pub const LAUNCHD_LABEL: &str = "dev.agend.daemon";
pub const SYSTEMD_UNIT: &str = "agend-daemon.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Manager {
    Launchd,
    Systemd,
}

impl Manager {
    pub fn filename(self) -> &'static str {
        match self {
            Self::Launchd => "dev.agend.daemon.plist",
            Self::Systemd => SYSTEMD_UNIT,
        }
    }
}

/// Explicit service environment: never capture tokens or an agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpec {
    pub manager: Manager,
    pub program: String,
    pub home: String,
    pub user_home: String,
    pub search_path: String,
}

impl ServiceSpec {
    pub fn render(&self) -> Result<String, &'static str> {
        for path in [&self.program, &self.home, &self.user_home] {
            if !absolute(path) {
                return Err("service paths must be absolute and contain no control characters");
            }
        }
        if self.search_path.split(':').any(|path| !absolute(path)) {
            return Err("service PATH must contain only nonempty absolute directories");
        }
        Ok(match self.manager {
            Manager::Launchd => self.launchd(),
            Manager::Systemd => self.systemd(),
        })
    }

    fn launchd(&self) -> String {
        let program = xml(&self.program);
        let home = xml(&self.home);
        let user_home = xml(&self.user_home);
        let path = xml(&self.search_path);
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<plist version=\"1.0\"><dict>\n\
<key>Label</key><string>{LAUNCHD_LABEL}</string>\n\
<key>ProgramArguments</key><array><string>{program}</string><string>daemon</string></array>\n\
<key>WorkingDirectory</key><string>{home}</string>\n\
<key>EnvironmentVariables</key><dict>\n\
<key>AGEND_HOME</key><string>{home}</string>\n\
<key>HOME</key><string>{user_home}</string>\n\
<key>PATH</key><string>{path}</string>\n\
</dict>\n\
<key>RunAtLoad</key><true/>\n\
<key>KeepAlive</key><true/>\n\
<key>AbandonProcessGroup</key><true/>\n\
<key>ThrottleInterval</key><integer>5</integer>\n\
<key>Umask</key><integer>63</integer>\n\
<key>StandardOutPath</key><string>{home}/logs/service.stdout.log</string>\n\
<key>StandardErrorPath</key><string>{home}/logs/service.stderr.log</string>\n\
</dict></plist>\n"
        )
    }

    fn systemd(&self) -> String {
        // ':' disables environment expansion. env execs the literal program without
        // a shell; systemd forbids quotes/backslashes in its first executable word.
        // '%' still needs escaping in every expanded unit value.
        let program = unit(&self.program);
        // WorkingDirectory consumes a raw path, not a shell-style quoted word.
        // A final slash preserves trailing spaces/backslashes in directory names.
        let home = self.home.replace('%', "%%");
        let agend_home = unit(&format!("AGEND_HOME={}", self.home));
        let user_home = unit(&format!("HOME={}", self.user_home));
        let path = unit(&format!("PATH={}", self.search_path));
        format!(
            "[Unit]\nDescription=AgEnD agent daemon\n\n\
[Service]\nType=exec\nExecStart=:/usr/bin/env -- {program} daemon\n\
WorkingDirectory={home}/\nEnvironment={agend_home}\n\
Environment={user_home}\nEnvironment={path}\nUnsetEnvironment=AGEND_INSTANCE\n\
Restart=always\nRestartSec=5\nKillMode=process\nTimeoutStopSec=20\nUMask=0077\n\n\
[Install]\nWantedBy=default.target\n"
        )
    }
}

fn absolute(value: &str) -> bool {
    value.starts_with('/')
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{fffe}' | '\u{ffff}'))
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn unit(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(manager: Manager) -> ServiceSpec {
        ServiceSpec {
            manager,
            program: "/opt/Ag EnD/$bin%/agend".into(),
            home: "/tmp/a & <b> \"c\" 'd'".into(),
            user_home: "/home/繁中".into(),
            search_path: "/usr/bin:/bin".into(),
        }
    }

    #[test]
    fn service_values_cannot_inject_new_directives() {
        for manager in [Manager::Launchd, Manager::Systemd] {
            let mut value = spec(manager);
            value.home.push_str("\nExecStart=/bad");
            assert!(value.render().is_err());
            value = spec(manager);
            value.search_path = "/bin:relative".into();
            assert!(value.render().is_err());
            value.search_path = "/bin:".into();
            assert!(value.render().is_err());
            value.search_path = "/bin".into();
            value.program = "relative".into();
            assert!(value.render().is_err());
        }
    }

    #[test]
    fn services_preserve_holders_and_literal_paths() {
        let launchd = spec(Manager::Launchd).render().unwrap();
        assert!(launchd.contains("<key>AbandonProcessGroup</key><true/>"));
        assert!(launchd.contains("/tmp/a &amp; &lt;b&gt; &quot;c&quot; &apos;d&apos;"));
        assert!(launchd.contains("<string>/home/繁中</string>"));
        let systemd = spec(Manager::Systemd).render().unwrap();
        assert!(
            systemd.contains("ExecStart=:/usr/bin/env -- \"/opt/Ag EnD/$bin%%/agend\" daemon\n")
        );
        assert!(systemd.contains("KillMode=process\n"));
        assert!(systemd.contains("Environment=\"AGEND_HOME=/tmp/a & <b> \\\"c\\\" 'd'\"\n"));
        assert!(systemd.contains("UnsetEnvironment=AGEND_INSTANCE\n"));
    }
}
