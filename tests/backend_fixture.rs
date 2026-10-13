//! Original, non-media fixture for native process-tree ownership tests only.
use std::{env, fs, process::Command, thread, time::Duration};
fn main() {
    let a: Vec<String>=env::args().skip(1).collect();
    match a[0].as_str() {
        "lease" => {
            Command::new(env::current_exe().unwrap()).args(["child", &a[4]]).spawn().unwrap();
            fs::write(&a[1], std::process::id().to_string()).unwrap();
            while !std::path::Path::new(&a[2]).exists() { thread::sleep(Duration::from_millis(10)); }
            fs::write(&a[3], "backend completed").unwrap();
        },
        "sleep" => thread::sleep(Duration::from_secs(30)),
        "child" => { thread::sleep(Duration::from_secs(2)); fs::write(&a[1],"orphan survived").unwrap(); },
        "orphan" => { Command::new(env::current_exe().unwrap()).args(["child", &a[1]]).spawn().unwrap(); },
        "memory" => {
            let mut chunks=Vec::new();
            for _ in 0..32 {
                let mut block=Vec::<u8>::new();
                if block.try_reserve_exact(4*1024*1024).is_err() {
                    println!("fixture_allocation_denied");
                    std::process::exit(17);
                }
                block.resize(4*1024*1024, 0);
                for n in (0..block.len()).step_by(4096) { block[n]=17; }
                chunks.push(block);
                thread::sleep(Duration::from_millis(25));
            }
            thread::sleep(Duration::from_secs(1));
            std::hint::black_box(chunks);
        },
        "rss" => { let mut bytes=vec![0u8; 16*1024*1024]; for n in (0..bytes.len()).step_by(4096) { bytes[n]=17; } thread::sleep(Duration::from_millis(250)); std::hint::black_box(bytes); },
        _ => panic!("unknown fixture operation"),
    }
}
