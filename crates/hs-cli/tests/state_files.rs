use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn file_resume_matches_continuous_execution_and_never_overwrites_inputs() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("hachistep-state-{}-{nonce}", std::process::id());
    let dir = Scratch(std::env::temp_dir().join(name));
    fs::create_dir(&dir.0).unwrap();
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&[1, 0]);
    rom[0x100..0x108].copy_from_slice(&[0xf8, 0x42, 0x6a, 0x88, 0xf7, 0x80, 0x40, 0xfe]);
    fs::write(dir.0.join("rom.bin"), &rom).unwrap();
    fs::write(dir.0.join("save.bin"), [0xff; 65536]).unwrap();
    let run = |args: &[&str], succeeds| {
        let output = Command::new(env!("CARGO_BIN_EXE_hachistep"))
            .args(args)
            .current_dir(&dir.0)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            succeeds,
            "args={args:?}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(
        &[
            "run",
            "--firmware",
            "rom.bin",
            "--eeprom",
            "save.bin",
            "--milliseconds",
            "1",
            "--save-state",
            "mid.state",
        ],
        true,
    );
    run(
        &[
            "run",
            "--load-state",
            "mid.state",
            "--milliseconds",
            "3",
            "--out",
            "resumed",
        ],
        true,
    );
    run(
        &[
            "run",
            "--firmware",
            "rom.bin",
            "--eeprom",
            "save.bin",
            "--milliseconds",
            "3",
            "--out",
            "continuous",
        ],
        true,
    );
    for file in [
        "state.bin",
        "flash.bin",
        "eeprom.bin",
        "ram.bin",
        "lcd-ram.bin",
        "sensor-nv.bin",
        "frame.pgm",
    ] {
        assert_eq!(
            fs::read(dir.0.join("resumed").join(file)).unwrap(),
            fs::read(dir.0.join("continuous").join(file)).unwrap(),
            "{file}"
        );
    }
    run(
        &["run", "--load-state", "mid.state", "--firmware", "rom.bin"],
        false,
    );
    run(
        &["run", "--load-state", "save.bin", "--out", "invalid"],
        false,
    );
    assert!(!dir.0.join("invalid").exists());
    run(
        &["run", "--load-state", "mid.state", "--milliseconds", "0"],
        false,
    );
    run(
        &[
            "run",
            "--load-state",
            "mid.state",
            "--save-state",
            "rom.bin",
            "--milliseconds",
            "2",
        ],
        false,
    );
    assert_eq!(fs::read(dir.0.join("rom.bin")).unwrap(), rom);
    assert_eq!(fs::read(dir.0.join("save.bin")).unwrap(), [0xff; 65536]);
}
