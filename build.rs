// build.rs — Windows: 嵌入 requireAdministrator 清单（自动 UAC）+ 应用图标
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_manifest::embed_manifest(
            embed_manifest::new_manifest("l850-cops.admin").requested_execution_level(
                embed_manifest::manifest::ExecutionLevel::RequireAdministrator,
            ),
        )
        .expect("嵌入管理员清单失败");
        // 应用图标（标题栏/任务栏/资源管理器）
        embed_resource::compile("app.rc", embed_resource::NONE);
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=app.ico");
        println!("cargo:rerun-if-changed=build.rs");
    }
}
