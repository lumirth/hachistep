//! Host-side file safety: no third-party dependencies or private firmware.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "hs-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        let mut rom = vec![0; 49152];
        rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
        rom[0x100..0x102].copy_from_slice(&[0x40, 0xfe]); // BRA self
        fs::write(p.join("rom.bin"), rom).unwrap();
        fs::write(p.join("eep.bin"), [0xff; 65536]).unwrap();
        Self(p)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hachistep"))
            .current_dir(&self.0)
            .args([
                "run",
                "--firmware",
                "rom.bin",
                "--eeprom",
                "eep.bin",
                "--milliseconds",
                "1",
            ])
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn output_and_trace_never_replace_existing_input_files() {
    let t = Temp::new();
    let rom = fs::read(t.0.join("rom.bin")).unwrap();
    let eep = fs::read(t.0.join("eep.bin")).unwrap();
    assert!(!t.run(&["--frame", "rom.bin"]).status.success());
    assert!(!t.run(&["--trace", "eep.bin"]).status.success());
    fs::create_dir(t.0.join("existing")).unwrap();
    fs::write(t.0.join("existing/precious"), b"preserve").unwrap();
    assert!(!t.run(&["--out", "existing"]).status.success());
    assert_eq!(
        fs::read(t.0.join("existing/precious")).unwrap(),
        b"preserve"
    );
    assert_eq!(fs::read(t.0.join("rom.bin")).unwrap(), rom);
    assert_eq!(fs::read(t.0.join("eep.bin")).unwrap(), eep);
}
#[test]
fn malformed_inputs_fail_without_creating_output_directories() {
    let t = Temp::new();
    fs::write(t.0.join("bad.csv"), "1,ir,1\n0,ir,0\n").unwrap();
    assert!(!t
        .run(&["--input", "bad.csv", "--out", "result"])
        .status
        .success());
    assert!(!t.0.join("result").exists());
    assert!(!t
        .run(&["--status", "1", "--out", "result"])
        .status
        .success());
    assert!(!t.0.join("result").exists());
    fs::write(t.0.join("rom.bin"), b"short").unwrap();
    assert!(!t.run(&["--out", "result"]).status.success());
    assert!(!t.0.join("result").exists());
}
#[test]
fn independent_output_contains_replay_metadata_and_nonvolatile_images() {
    let t = Temp::new();
    fs::write(t.0.join("inputs.csv"), "0,buttons,0,0,0\n").unwrap();
    let output = t.run(&[
        "--input",
        "inputs.csv",
        "--out",
        "result",
        "--trace",
        "trace.txt",
        "--trace-limit",
        "0",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fs::read_to_string(t.0.join("result/report.json")).unwrap();
    for key in [
        "\"schema\": 2",
        "\"input_sha256\": \"",
        "\"initial_sensor_nv_sha256\": \"",
        "\"initial_conditions\":",
        "\"fault\": null",
    ] {
        assert!(report.contains(key), "missing {key}");
    }
    for (name, bytes) in [
        ("eeprom.bin", 65536),
        ("eeprom.status", 1),
        ("sensor-nv.bin", 19),
        ("ram.bin", 2048),
        ("lcd-ram.bin", 4096),
    ] {
        assert_eq!(
            fs::metadata(t.0.join("result").join(name)).unwrap().len(),
            bytes
        );
    }
    assert!(t
        .run(&["--sensor-nv", "result/sensor-nv.bin"])
        .status
        .success());
}
