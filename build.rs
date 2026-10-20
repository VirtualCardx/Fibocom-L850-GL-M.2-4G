// build.rs — Windows: 嵌入 requireAdministrator 清单，启动时自动弹 UAC
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_manifest::embed_manifest(
            embed_manifest::new_manifest("l850-cops.admin")
                .requested_execution_level(
                embed_manifest::manifest::ExecutionLevel::RequireAdministrator,
                ),
        )
        .expect("嵌入管理员清单失败");
        println!("cargo:rerun-if-changed=build.rs");
    }
}
