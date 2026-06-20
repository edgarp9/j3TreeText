fn main() {
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    println!("cargo:rerun-if-changed=icon.ico");

    if matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("windows")
    ) {
        embed_resource::compile_for("resources/app.rc", ["j3TreeText"], embed_resource::NONE)
            .manifest_optional()
            .unwrap_or_else(|error| panic!("failed to compile Windows resources: {error}"));
    }
}
