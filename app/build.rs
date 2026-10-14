// ----------------------------------------------------------------------------
fn main() {
    println!("cargo:rerun-if-changed=avelor.rc");
    println!("cargo:rerun-if-changed=res");

    let _ = embed_resource::compile("avelor.rc", embed_resource::NONE);
    static_vcruntime::metabuild();
}
