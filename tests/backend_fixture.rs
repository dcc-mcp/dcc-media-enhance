//! Original, non-media fixture for native process-tree ownership tests only.
use std::{env, fs, process::Command, thread, time::Duration};
fn main() {
    let a: Vec<String>=env::args().skip(1).collect();
    match a[0].as_str() {
        "sleep" => thread::sleep(Duration::from_secs(30)),
        "child" => { thread::sleep(Duration::from_secs(2)); fs::write(&a[1],"orphan survived").unwrap(); },
        "orphan" => { Command::new(env::current_exe().unwrap()).args(["child", &a[1]]).spawn().unwrap(); },
        "rss" => { let mut bytes=vec![0u8; 16*1024*1024]; for n in (0..bytes.len()).step_by(4096) { bytes[n]=17; } thread::sleep(Duration::from_millis(250)); std::hint::black_box(bytes); },
        _ => panic!("unknown fixture operation"),
    }
}
