fn main() {
    println!("cargo:rerun-if-env-changed=DCC_MEDIA_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    let sha=std::env::var("DCC_MEDIA_BUILD_COMMIT").or_else(|_|std::env::var("GITHUB_SHA")).unwrap_or_else(|_|"unknown-local-build".into());
    assert!(!sha.contains(['\r','\n']), "Invalid source commit metadata");
    println!("cargo:rustc-env=DCC_MEDIA_BUILD_COMMIT={sha}");
    println!("cargo:rustc-env=DCC_MEDIA_TARGET={}",std::env::var("TARGET").unwrap());
}
