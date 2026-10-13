use crate::{contract::{self, Limits}, process};
use std::{fs, path::PathBuf, process::Command, sync::OnceLock, thread, time::{Duration, Instant}};
fn fixture() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir=std::env::temp_dir().join(format!("dcc-native-fixture-{}",std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let binary=dir.join(if cfg!(windows) {"backend.exe"} else {"backend"});
        assert!(Command::new("rustc").args(["--edition=2024","tests/backend_fixture.rs","-o"]).arg(&binary).status().unwrap().success());
        binary
    })
}
fn work(name:&str)->PathBuf {
    let dir=std::env::temp_dir().join(format!("dcc-native-{name}-{}-{}",std::process::id(),contract::now_ms()));
    fs::create_dir(&dir).unwrap(); dir
}
#[test]
fn native_timeout_terminates_backend() {
    let dir=work("timeout"); let mut limits=Limits::inspection(); limits.timeout_secs=1;
    let began=Instant::now();
    let err=process::run(fixture(), &["sleep".into()],&limits,&dir.join("log"),None,None).unwrap_err();
    assert_eq!(err.code,"TIMEOUT"); assert!(began.elapsed()<Duration::from_secs(8));
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn native_live_sentinel_cancels_backend() {
    let dir=work("cancel"); let cancel=dir.join("cancel"); let worker=cancel.clone();
    let binary=fixture();
    let writer=thread::spawn(move || {thread::sleep(Duration::from_millis(150));fs::write(worker,"cancel").unwrap();});
    let err=process::run(binary,&["sleep".into()],&Limits::inspection(),&dir.join("log"),None,Some(&cancel)).unwrap_err();
    writer.join().unwrap(); assert_eq!(err.code,"CANCELLED"); fs::remove_dir_all(dir).unwrap();
}
#[test]
fn native_leader_exit_stops_owned_descendant() {
    let dir=work("descendant"); let marker=dir.join("survived");
    process::run(fixture(),&["orphan".into(),marker.to_string_lossy().into_owned()],&Limits::inspection(),&dir.join("log"),None,None).unwrap();
    thread::sleep(Duration::from_secs(3)); assert!(!marker.exists(),"Owned descendant escaped cleanup"); fs::remove_dir_all(dir).unwrap();
}
#[test]
fn native_rss_and_disk_are_real_observations() {
    let dir=work("resources");
    let receipt=process::run(fixture(),&["rss".into()],&Limits::inspection(),&dir.join("log"),None,None).unwrap();
    assert!(receipt.sampled_peak_rss_kib>=8*1024, "RSS probe did not observe allocated pages");
    assert!(process::disk_free(&dir).unwrap()>0);
    assert!(process::process_identity(std::process::id()).is_some()); fs::remove_dir_all(dir).unwrap();
}
