fn main() {
    // Embed the Windows application manifest so the executable depends on Common
    // Controls v6. tray-icon's `common-controls-v6` feature statically imports
    // TaskDialogIndirect from comctl32 v6, which the loader only resolves when an
    // activation context from this manifest is present; otherwise the standalone
    // exe aborts at startup with "entry point TaskDialogIndirect could not be
    // located". No-op when not targeting Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/agent-notify-bridge.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed Windows application manifest");
    }
}
