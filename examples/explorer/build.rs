//! Embeds the Windows application manifest into the Explorer consumer.

fn main() {
    #[cfg(target_os = "windows")]
    {
        windows_reactor_setup::as_self_contained();
        embed_resource::compile(
            "../../packaging/windows/rinka-explorer.rc",
            embed_resource::NONE,
        )
        .manifest_required()
        .expect("Windows application resources must compile");
    }
}
