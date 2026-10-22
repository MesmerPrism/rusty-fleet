// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

// Same rustc/exact optional lld pair used by the existing connectivity owner tests.
fn fixture_compiler() -> Command {
    let mut compiler = Command::new("rustc");
    match (
        std::env::var_os("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER"),
        std::env::var_os("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS"),
    ) {
        (None, None) => {}
        (Some(linker), Some(flags)) => {
            assert!(Path::new(&linker).is_absolute());
            assert_eq!(flags, "-Clinker-flavor=lld-link");
            let mut argument = std::ffi::OsString::from("linker=");
            argument.push(linker);
            compiler
                .arg("-C")
                .arg(argument)
                .args(["-C", "linker-flavor=lld-link"]);
        }
        _ => panic!("incomplete fixture compiler configuration"),
    }
    compiler
}
#[test]
fn real_windows_provider_process_bounds_exit_eof_and_descendant_cleanup() {
    let directory = std::env::temp_dir().join(format!(
        "fleet-kiosk-provider-process-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).expect("isolated fixture directory");
    let source = directory.join("fixture.rs");
    std::fs::write(&source, r#"
use std::{io::{Read,Write}, process::{Command,Stdio}, thread, time::Duration};
fn main() {
    if std::env::args().nth(1).as_deref() == Some("descendant") {
        loop { std::fs::write("heartbeat", b"alive").expect("heartbeat"); thread::sleep(Duration::from_millis(10)); }
    }
    assert_eq!(std::env::args().skip(1).collect::<Vec<_>>(), ["integration", "kiosk-v2-catalog", "--json"]);
    let mut input=String::new(); std::io::stdin().read_to_string(&mut input).expect("request EOF");
    match input.as_str() {
        "success" => { print!("complete"); },
        "unavailable" => { print!("unavailable"); std::process::exit(3); },
        "stdout" => { let _ = std::io::stdout().write_all(&vec![b'x'; 8*1024*1024+1]); },
        "stderr" => { let _ = std::io::stderr().write_all(&vec![b'x'; 16*1024+1]); },
        "warning" => { eprint!("unexportable owner context"); },
        "hang" => { loop { thread::sleep(Duration::from_secs(1)); } },
        "leaked-eof" => { Command::new(std::env::current_exe().expect("exe")).arg("descendant").stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn().expect("descendant"); },
        _ => panic!("unexpected mode"),
    }
}
"#).expect("source");
    let executable = directory.join(PROVIDER_EXECUTABLE_NAME);
    let output = fixture_compiler()
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("fixture compiler");
    assert!(
        output.status.success(),
        "fixture compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut runner = BoundedProviderProcessRunner {
        timeout: Duration::from_millis(750),
    };
    let success = runner
        .run(&executable, "profile.fixture.001", b"success")
        .expect("real process success");
    assert_eq!(success.exit_code, 0);
    assert_eq!(success.stdout, b"complete");
    assert_eq!(
        runner
            .run(&executable, "profile.fixture.001", b"unavailable")
            .expect("real unavailable process")
            .exit_code,
        3
    );
    assert_eq!(
        runner.run(&executable, "profile.fixture.001", b"stdout"),
        Err("provider_stdout_too_large")
    );
    assert_eq!(
        runner.run(&executable, "profile.fixture.001", b"stderr"),
        Err("provider_stderr_too_large")
    );
    assert_eq!(
        runner.run(&executable, "profile.fixture.001", b"warning"),
        Err("provider_stderr_not_empty")
    );
    assert_eq!(
        runner.run(&executable, "profile.fixture.001", b"hang"),
        Err("provider_timeout")
    );
    assert_eq!(
        runner.run(&executable, "profile.fixture.001", b"leaked-eof"),
        Err("provider_timeout")
    );
    let marker = directory.join("heartbeat");
    let modified = std::fs::metadata(&marker)
        .expect("descendant ran")
        .modified()
        .expect("time");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        std::fs::metadata(marker)
            .expect("marker")
            .modified()
            .expect("time"),
        modified,
        "owned descendant survived Job Object cleanup"
    );
    std::fs::remove_dir_all(directory).expect("cleanup exact isolated fixture");
}
