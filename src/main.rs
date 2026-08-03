//! Dynamic (subprocess) entrypoint for the jellyfin plugin.
//!
//! The toolkit's `serve_tool_plugin!` emits `fn main`, serving this plugin over the orca
//! socket. The plugin is a
//! `[[bin]]`, owns no runtime, and reaches orca only through the socket.
//!
//! `link: jellyfin` force-links this plugin's own lib crate so its
//! `#[orca_tool]` inventory survives linking — without it the `[[bin]]`
//! references nothing in the rlib and the linker drops every tool.
plugin_toolkit::serve_tool_plugin! {
    name: "jellyfin",
    target_compat: "10.8-10.10",
    link: jellyfin,
}
