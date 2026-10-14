use std::env;
use std::fs;
use std::path::{Path, PathBuf};

// ----------------------------------------------------------------------------
/// The manifest's version (`res/avelor.exe.manifest`, `assemblyIdentity`),
/// which the resource compiler takes as it is: the workspace's with `.0`,
/// or the build fails.
fn check_manifest(root: &Path) {
    let path = root.join("res/avelor.exe.manifest");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let identity = text
        .split_once("<assemblyIdentity")
        .and_then(|(_, rest)| rest.split_once("/>"))
        .map(|(identity, _)| identity)
        .expect("res/avelor.exe.manifest has no assemblyIdentity");
    let version = identity
        .split_once("version=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(version, _)| version)
        .expect("res/avelor.exe.manifest's assemblyIdentity has no version");
    let want = format!("{}.0", env::var("CARGO_PKG_VERSION").unwrap());
    assert!(
        version == want,
        "res/avelor.exe.manifest: assemblyIdentity's version is {version}, not {want}, the \
         workspace's version"
    );
}

// ----------------------------------------------------------------------------
/// The resource compiler's macros for `avelor.rc`'s VERSIONINFO: the
/// version's numbers, and its text, which the compiler can't make of them.
fn version_macros() -> Vec<String> {
    let part = |name: &str| env::var(format!("CARGO_PKG_VERSION_{name}")).unwrap();
    vec![
        format!("VERSION_MAJOR={}", part("MAJOR")),
        format!("VERSION_MINOR={}", part("MINOR")),
        format!("VERSION_PATCH={}", part("PATCH")),
        format!(
            "VERSION_TEXT=\"{}\"",
            env::var("CARGO_PKG_VERSION").unwrap()
        ),
    ]
}

// ----------------------------------------------------------------------------
fn main() {
    println!("cargo:rerun-if-changed=avelor.rc");
    println!("cargo:rerun-if-changed=res");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    check_manifest(&root);

    let _ = embed_resource::compile("avelor.rc", version_macros());
    static_vcruntime::metabuild();
}
