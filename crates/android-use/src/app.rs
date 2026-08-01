use serde_json::{json, Value};

use crate::adb::Adb;
use crate::error::{AuError, Result};
use crate::process::text;

pub fn execute(adb: &Adb, serial: &str, args: &[String]) -> Result<Value> {
    let operation = args.first().map(String::as_str).unwrap_or("ls");
    match operation {
        "ls" => list(adb, serial),
        "info" => info(adb, serial, required(args, 1, "app info PACKAGE")?),
        "start" => start(adb, serial, &args[1..]),
        "stop" => force_stop(adb, serial, required(args, 1, "app stop PACKAGE")?),
        "install" => install(adb, serial, required(args, 1, "app install APK")?),
        "uninstall" => uninstall(adb, serial, required(args, 1, "app uninstall PACKAGE")?),
        "clear" => clear(adb, serial, required(args, 1, "app clear PACKAGE")?),
        "perm" => permissions(adb, serial, required(args, 1, "app perm PACKAGE")?),
        "grant" => permission_change(
            adb,
            serial,
            "grant",
            required(args, 1, "app grant PACKAGE PERMISSION")?,
            required(args, 2, "app grant PACKAGE PERMISSION")?,
        ),
        "revoke" => permission_change(
            adb,
            serial,
            "revoke",
            required(args, 1, "app revoke PACKAGE PERMISSION")?,
            required(args, 2, "app revoke PACKAGE PERMISSION")?,
        ),
        "intent" => intent(adb, serial, &args[1..]),
        _ => Err(AuError::code(
            "E_ARGS",
            format!("unknown app operation {operation}"),
        )),
    }
}

fn list(adb: &Adb, serial: &str) -> Result<Value> {
    let result = adb.device(
        serial,
        &[
            "shell".into(),
            "pm".into(),
            "list".into(),
            "packages".into(),
        ],
    )?;
    let packages = text(&result.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("package:"))
        .take(2_000)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok(json!({"count":packages.len(),"packages":packages}))
}

fn info(adb: &Adb, serial: &str, package: &str) -> Result<Value> {
    let result = adb.device(
        serial,
        &[
            "shell".into(),
            "dumpsys".into(),
            "package".into(),
            package.into(),
        ],
    )?;
    Ok(json!({"package":package,"text":limited(text(&result.stdout), 12_000)}))
}

fn start(adb: &Adb, serial: &str, args: &[String]) -> Result<Value> {
    let package = args
        .first()
        .ok_or_else(|| AuError::code("E_ARGS", "app start PACKAGE [ACTIVITY]"))?;
    let command = if let Some(activity) = args.get(1) {
        vec![
            "shell".into(),
            "am".into(),
            "start".into(),
            "-W".into(),
            "-n".into(),
            format!("{package}/{activity}"),
        ]
    } else {
        vec![
            "shell".into(),
            "am".into(),
            "start".into(),
            "-W".into(),
            "-a".into(),
            "android.intent.action.MAIN".into(),
            "-c".into(),
            "android.intent.category.LAUNCHER".into(),
            "-p".into(),
            package.clone(),
        ]
    };
    let result = adb.device(serial, &command)?;
    let proof = text(&result.stdout);
    if launch_rejected(&proof) {
        return Err(AuError::code(
            "E_APP",
            format!(
                "Android rejected launch for {package}: {}",
                limited(proof, 400)
            ),
        ));
    }
    Ok(json!({"package":package,"started":true,"proof":limited(proof, 400)}))
}

fn force_stop(adb: &Adb, serial: &str, package: &str) -> Result<Value> {
    adb.device(
        serial,
        &[
            "shell".into(),
            "am".into(),
            "force-stop".into(),
            package.into(),
        ],
    )?;
    Ok(json!({"package":package,"stopped":true}))
}

fn install(adb: &Adb, serial: &str, apk: &str) -> Result<Value> {
    adb.device(serial, &["install".into(), "-r".into(), apk.into()])?;
    Ok(json!({"apk":apk,"installed":true}))
}

fn uninstall(adb: &Adb, serial: &str, package: &str) -> Result<Value> {
    adb.device(serial, &["uninstall".into(), package.into()])?;
    Ok(json!({"package":package,"uninstalled":true}))
}

fn clear(adb: &Adb, serial: &str, package: &str) -> Result<Value> {
    adb.device(
        serial,
        &["shell".into(), "pm".into(), "clear".into(), package.into()],
    )?;
    Ok(json!({"package":package,"cleared":true}))
}

fn permissions(adb: &Adb, serial: &str, package: &str) -> Result<Value> {
    let result = adb.device(
        serial,
        &[
            "shell".into(),
            "dumpsys".into(),
            "package".into(),
            package.into(),
        ],
    )?;
    let lines = text(&result.stdout)
        .lines()
        .filter(|line| line.contains("permission") || line.contains("granted="))
        .take(200)
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok(json!({"package":package,"permissions":lines}))
}

fn permission_change(
    adb: &Adb,
    serial: &str,
    operation: &str,
    package: &str,
    permission: &str,
) -> Result<Value> {
    adb.device(
        serial,
        &[
            "shell".into(),
            "pm".into(),
            operation.into(),
            package.into(),
            permission.into(),
        ],
    )?;
    Ok(json!({"package":package,"permission":permission,"operation":operation}))
}

fn intent(adb: &Adb, serial: &str, arguments: &[String]) -> Result<Value> {
    let action = arguments.first().ok_or_else(|| {
        AuError::code(
            "E_ARGS",
            "app intent ACTION [--data URI] [--component PKG/CLASS] [--extra-string KEY VALUE]",
        )
    })?;
    let mut command = vec![
        "shell".into(),
        "am".into(),
        "start".into(),
        "-a".into(),
        action.clone(),
    ];
    let mut index = 1usize;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--data" => {
                command.extend([
                    "-d".into(),
                    required(arguments, index + 1, "--data URI")?.into(),
                ]);
                index += 2;
            }
            "--component" => {
                command.extend([
                    "-n".into(),
                    required(arguments, index + 1, "--component PKG/CLASS")?.into(),
                ]);
                index += 2;
            }
            "--extra-string" => {
                command.extend([
                    "--es".into(),
                    required(arguments, index + 1, "--extra-string KEY VALUE")?.into(),
                    required(arguments, index + 2, "--extra-string KEY VALUE")?.into(),
                ]);
                index += 3;
            }
            value => {
                return Err(AuError::code(
                    "E_ARGS",
                    format!("unsupported structured intent option {value}"),
                ))
            }
        }
    }
    adb.device(serial, &command)?;
    Ok(json!({"action":action,"started":true}))
}

fn required<'a>(args: &'a [String], index: usize, usage: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| AuError::code("E_ARGS", usage))
}

fn limited(value: String, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn launch_rejected(proof: &str) -> bool {
    proof.contains("Error type") || proof.contains("Error:") || proof.contains("Exception")
}

#[cfg(test)]
mod tests {
    use super::launch_rejected;

    #[test]
    fn app_launch_rejects_android_error_text_even_with_zero_adb_exit() {
        assert!(launch_rejected(
            "Starting: Intent { cmp=x/.Missing }\nError type 3\nError: Activity class does not exist."
        ));
        assert!(!launch_rejected("Status: ok\nActivity: x/.MainActivity"));
    }
}
