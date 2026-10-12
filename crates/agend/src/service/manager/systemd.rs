//! Validate the loaded D-Bus properties, not merely the unit's on-disk path.
//! busctl preserves argument boundaries and type information in JSON.

use agend_core::setup::service::Installation;
use serde_json::{Value, json};

use super::{State, run_program, run_program_until};
use std::time::{Duration, Instant};

const OBJECT: &str = "/org/freedesktop/systemd1/unit/agend_2ddaemon_2eservice";

pub(super) fn inspect(record: &Installation) -> Result<State, String> {
    wait_for_user_bus()?;
    let unit = properties("org.freedesktop.systemd1.Unit")?;
    let service = properties("org.freedesktop.systemd1.Service")?;
    let state = validate(record, &unit, &service)?;
    let pid = property(&service, "MainPID", "u")?
        .as_u64()
        .ok_or("invalid systemd MainPID")?;
    if pid != 0 {
        live_process(record, pid)?;
    }
    Ok(state)
}

fn wait_for_user_bus() -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    wait_for_name_owner(deadline, |deadline| {
        run_program_until(
            "/usr/bin/busctl",
            &[
                "--user",
                "--json=short",
                "--auto-start=no",
                "--allow-interactive-authorization=no",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                "s",
                "org.freedesktop.systemd1",
            ],
            deadline,
        )
    })
}

fn wait_for_name_owner(
    deadline: Instant,
    mut probe: impl FnMut(Instant) -> Result<super::Reply, String>,
) -> Result<(), String> {
    loop {
        if Instant::now() >= deadline {
            return Err("systemd user bus readiness deadline expired; preserving service".into());
        }
        let reply = probe(deadline)?;
        if reply.code != 0 {
            return Err("cannot inspect user bus readiness; preserving service".into());
        }
        let ready = name_has_owner(&reply.stdout)?;
        if Instant::now() >= deadline {
            return Err("systemd user bus readiness deadline expired; preserving service".into());
        }
        if ready {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("systemd user bus is not ready; preserving service".into());
        }
        std::thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

fn name_has_owner(output: &str) -> Result<bool, String> {
    let value: Value =
        serde_json::from_str(output).map_err(|_| "invalid user bus readiness JSON")?;
    if value["type"] != "b" || value["data"].as_array().is_none_or(|a| a.len() != 1) {
        return Err("unexpected user bus readiness envelope".into());
    }
    value["data"][0]
        .as_bool()
        .ok_or_else(|| "invalid user bus readiness boolean".into())
}

fn properties(interface: &str) -> Result<Value, String> {
    let reply = run_program(
        "/usr/bin/busctl",
        &[
            "--user",
            "--json=short",
            "--auto-start=no",
            "--allow-interactive-authorization=no",
            "call",
            "org.freedesktop.systemd1",
            OBJECT,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            "s",
            interface,
        ],
    )?;
    if reply.code != 0 {
        return Err("cannot inspect loaded systemd properties; preserving service".into());
    }
    let value: Value =
        serde_json::from_str(&reply.stdout).map_err(|_| "invalid systemd property JSON")?;
    if value["type"] != "a{sv}"
        || value["data"].as_array().is_none_or(|a| a.len() != 1)
        || !value["data"][0].is_object()
    {
        return Err("unexpected systemd property envelope".into());
    }
    Ok(value["data"][0].clone())
}

fn property<'a>(object: &'a Value, name: &str, signature: &str) -> Result<&'a Value, String> {
    let value = &object[name];
    if value["type"] != signature || value.get("data").is_none() {
        return Err(format!(
            "systemd property {name} is missing or has an unexpected type"
        ));
    }
    Ok(&value["data"])
}

fn require(object: &Value, name: &str, signature: &str, expected: Value) -> Result<(), String> {
    if property(object, name, signature)? != &expected {
        return Err(format!(
            "loaded systemd property {name} differs; preserving service"
        ));
    }
    Ok(())
}

fn validate(record: &Installation, unit: &Value, service: &Value) -> Result<State, String> {
    require(unit, "FragmentPath", "s", json!(record.service_path))?;
    require(unit, "LoadState", "s", json!("loaded"))?;
    require(unit, "DropInPaths", "as", json!([]))?;
    require(unit, "Transient", "b", json!(false))?;
    for (name, value) in [
        ("Type", "exec"),
        ("KillMode", "process"),
        ("Restart", "always"),
        ("RootDirectory", ""),
        ("RootImage", ""),
        ("User", ""),
        ("Group", ""),
    ] {
        require(service, name, "s", json!(value))?;
    }
    require(service, "KillSignal", "i", json!(15))?;
    require(service, "EnvironmentFiles", "a(sb)", json!([]))?;
    require(service, "PassEnvironment", "as", json!([]))?;
    require(service, "UnsetEnvironment", "as", json!(["AGEND_INSTANCE"]))?;
    let directory = property(service, "WorkingDirectory", "s")?;
    if directory != &json!(record.spec.home)
        && directory != &json!(format!("{}/", record.spec.home))
    {
        return Err("loaded systemd working directory differs; preserving service".into());
    }
    let mut expected = vec![
        format!("AGEND_HOME={}", record.spec.home),
        format!("HOME={}", record.spec.user_home),
        format!("PATH={}", record.spec.search_path),
    ];
    let mut found: Vec<String> =
        serde_json::from_value(property(service, "Environment", "as")?.clone())
            .map_err(|_| "invalid systemd environment")?;
    expected.sort();
    found.sort();
    if found != expected {
        return Err("loaded systemd environment differs; preserving service".into());
    }
    for name in [
        "ExecConditionEx",
        "ExecStartPreEx",
        "ExecStartPostEx",
        "ExecStopEx",
        "ExecStopPostEx",
    ] {
        require(service, name, "a(sasasttttuii)", json!([]))?;
    }
    let start = property(service, "ExecStartEx", "a(sasasttttuii)")?
        .as_array()
        .ok_or("invalid systemd ExecStartEx")?;
    if start.len() != 1
        || start[0].as_array().is_none_or(|entry| entry.len() != 10)
        || start[0][0] != "/usr/bin/env"
        || start[0][1] != json!(["/usr/bin/env", "--", record.spec.program, "daemon"])
        || start[0][2] != json!(["no-env-expand"])
    {
        return Err("loaded systemd command differs; preserving service".into());
    }
    let pid = property(service, "MainPID", "u")?
        .as_u64()
        .ok_or("invalid systemd MainPID")?;
    let state = property(unit, "ActiveState", "s")?
        .as_str()
        .ok_or("invalid systemd ActiveState")?;
    Ok(State::Owned {
        running: pid != 0 || !matches!(state, "inactive" | "failed"),
    })
}

/// A daemon-reload changes the loaded command without restarting MainPID.
/// Verify the live executable, argv and home too; never print its environment.
fn live_process(record: &Installation, pid: u64) -> Result<(), String> {
    use std::fs;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    if pid <= 1 || pid > u32::MAX as u64 {
        return Err("invalid live service PID".into());
    }
    let path = std::path::PathBuf::from(format!("/proc/{pid}"));
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        fs::File::open(path.join(name))
            .and_then(|f| f.take(1_048_577).read_to_end(&mut bytes))
            .map_err(|_| "cannot inspect live service identity; preserving service")?;
        if bytes.len() > 1_048_576 {
            return Err("live service identity exceeds limit".into());
        }
        Ok(bytes)
    };
    let generation = |bytes: &[u8]| -> Result<String, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid service process stat")?;
        text.rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(19))
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .map(str::to_owned)
            .ok_or_else(|| "missing process start time".into())
    };
    let before = generation(&read("stat")?)?;
    let process = fs::metadata(&path).map_err(|_| "live service process disappeared")?;
    let actual = fs::metadata(path.join("exe")).map_err(|_| "cannot inspect live executable")?;
    let expected =
        fs::metadata(&record.spec.program).map_err(|_| "managed executable disappeared")?;
    // SAFETY: getuid only reads our real uid.
    if process.uid() != unsafe { libc::getuid() }
        || actual.dev() != expected.dev()
        || actual.ino() != expected.ino()
    {
        return Err(
            "live service executable belongs to another installation; preserving it".into(),
        );
    }
    let argv = read("cmdline")?;
    let wanted = format!("{}\0daemon\0", record.spec.program);
    let environment = read("environ")?;
    let homes: Vec<_> = environment
        .split(|&b| b == 0)
        .filter(|s| s.starts_with(b"AGEND_HOME="))
        .collect();
    let home = format!("AGEND_HOME={}", record.spec.home);
    if argv != wanted.as_bytes()
        || homes != [home.as_bytes()]
        || before != generation(&read("stat")?)?
    {
        return Err("live service arguments, home or generation differ; preserving it".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::setup::service::{InstallPhase, Manager, ServiceSpec};

    #[test]
    fn native_readiness_booleans_are_strictly_decoded() {
        let absent =
            include_str!("../../../tests/fixtures/systemd-effective/native-name-owner-false.json");
        let present =
            include_str!("../../../tests/fixtures/systemd-effective/native-name-owner-true.json");
        assert!(!name_has_owner(absent).unwrap());
        assert!(name_has_owner(present).unwrap());
        let native: Value = serde_json::from_str(absent).unwrap();
        for replacement in [json!("false"), json!(0), Value::Null] {
            let mut changed = native.clone();
            changed["data"][0] = replacement;
            assert!(name_has_owner(&changed.to_string()).is_err());
        }
        let mut changed = native.clone();
        changed["data"].as_array_mut().unwrap().push(json!(false));
        assert!(name_has_owner(&changed.to_string()).is_err());
        let mut changed = native;
        changed.as_object_mut().unwrap().remove("type");
        assert!(name_has_owner(&changed.to_string()).is_err());
        assert!(name_has_owner(&present[..present.len() / 2]).is_err());
    }

    fn native_readiness_reply(present: bool) -> super::super::Reply {
        super::super::Reply {
            code: 0,
            stdout: if present {
                include_str!(
                    "../../../tests/fixtures/systemd-effective/native-name-owner-true.json"
                )
            } else {
                include_str!(
                    "../../../tests/fixtures/systemd-effective/native-name-owner-false.json"
                )
            }
            .into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn readiness_waits_only_for_a_native_false_reply() {
        let mut calls = 0;
        wait_for_name_owner(Instant::now() + Duration::from_secs(2), |_| {
            calls += 1;
            Ok(native_readiness_reply(calls == 2))
        })
        .unwrap();
        assert_eq!(calls, 2);
        for failure in 0..3 {
            let mut calls = 0;
            let result = wait_for_name_owner(Instant::now() + Duration::from_secs(2), |_| {
                calls += 1;
                if failure == 0 {
                    return Err("native transport refused".into());
                }
                let mut reply = native_readiness_reply(false);
                if failure == 1 {
                    reply.code = 1;
                } else {
                    reply.stdout.truncate(reply.stdout.len() / 2);
                }
                Ok(reply)
            });
            assert!(result.is_err());
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn readiness_rejects_absence_and_success_after_the_deadline() {
        let started = Instant::now();
        assert!(
            wait_for_name_owner(started + Duration::from_millis(30), |_| {
                Ok(native_readiness_reply(false))
            })
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            wait_for_name_owner(Instant::now() + Duration::from_millis(10), |_| {
                std::thread::sleep(Duration::from_millis(30));
                Ok(native_readiness_reply(true))
            })
            .is_err()
        );
    }

    fn record() -> Installation {
        let home = "/tmp/g13 data % $value \"quote\"";
        Installation {
            version: 1,
            spec: ServiceSpec {
                manager: Manager::Systemd,
                program: format!("{home}/service/agend"),
                home: home.into(),
                user_home: "/tmp/g13-user".into(),
                search_path: "/usr/bin:/bin".into(),
            },
            service_path: "/root/.config/systemd/user/agend-daemon.service".into(),
            program_sha256: "0".repeat(64),
            phase: InstallPhase::Prepared,
        }
    }
    fn captured(source: &str) -> Value {
        let value: Value = serde_json::from_str(source).unwrap();
        assert_eq!(value["type"], "a{sv}");
        value["data"][0].clone()
    }
    const UNIT: &str = include_str!("../../../tests/fixtures/systemd-effective/native-unit.json");
    const SERVICE: &str =
        include_str!("../../../tests/fixtures/systemd-effective/native-service.json");
    const OVERRIDE_UNIT: &str =
        include_str!("../../../tests/fixtures/systemd-effective/override-unit.json");
    const OVERRIDE_SERVICE: &str =
        include_str!("../../../tests/fixtures/systemd-effective/override-service.json");

    #[test]
    fn native_systemd_properties_preserve_literal_argument_boundaries() {
        assert_eq!(
            validate(&record(), &captured(UNIT), &captured(SERVICE)).unwrap(),
            State::Owned { running: false }
        );
    }

    #[test]
    fn native_drop_in_and_each_changed_effective_property_are_refused() {
        let unit = captured(UNIT);
        let service = captured(SERVICE);
        let changed = captured(OVERRIDE_SERVICE);
        assert!(validate(&record(), &captured(OVERRIDE_UNIT), &service).is_err());
        for name in ["ExecStartEx", "ExecStopEx", "KillMode", "Environment"] {
            let mut one_change = service.clone();
            one_change[name] = changed[name].clone();
            assert!(validate(&record(), &unit, &one_change).is_err(), "{name}");
        }
        for name in service.as_object().unwrap().keys() {
            let mut missing = service.clone();
            missing.as_object_mut().unwrap().remove(name);
            assert!(
                validate(&record(), &unit, &missing).is_err(),
                "missing {name}"
            );
        }
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn live_process_identity_uses_the_actual_executable_arguments_and_home() {
        use std::process::{Child, Command, Stdio};
        struct Guard(Child);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let root = agend_testkit::tempdir::TempDir::new("g13-live-service").unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let program = root_path.join("agend");
        std::fs::copy("/bin/sh", &program).unwrap();
        // A real native process with exactly the daemon argv. read waits on the
        // parent's pipe without spawning any child or contacting any service.
        std::fs::write(root_path.join("daemon"), "echo ready > ready\nread value\n").unwrap();
        let child = Guard(
            Command::new(&program)
                .arg("daemon")
                .current_dir(&root_path)
                .env("AGEND_HOME", &root_path)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !root_path.join("ready").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "native identity fixture did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut expected = record();
        expected.spec.program = program.to_str().unwrap().into();
        expected.spec.home = root_path.to_str().unwrap().into();
        live_process(&expected, child.0.id().into()).unwrap();
        expected.spec.home.push_str("/foreign");
        assert!(live_process(&expected, child.0.id().into()).is_err());
        expected.spec.home = root_path.to_str().unwrap().into();
        expected.spec.program = "/bin/false".into();
        assert!(live_process(&expected, child.0.id().into()).is_err());
        drop(child);
    }
}
