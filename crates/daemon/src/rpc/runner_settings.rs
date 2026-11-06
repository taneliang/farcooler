use super::{DomainError, Result, Rpc, Service, agents_found_now, wire};

impl Rpc {
    // MARK: - Runner settings helpers

    /// This runner as it is right now, for a settings write to answer with.
    ///
    /// Named for the wire type it builds, not for the word a person reads —
    /// see `wire::host` for why `Host` stays `Host` on the wire.
    pub(super) async fn host_now(&self, svc: &Service) -> Result<farcooler_protocol::v1::Host> {
        svc.inventory.refresh().await;
        Ok(wire::host(
            &self.daemon_version,
            svc.host_id,
            &svc.inventory_snapshot(),
            0,
            crate::service::stand_in_agent(),
            crate::push::Pairing::load_in(svc.root_dir()).is_some(),
            svc.store.schema().ok(),
            agents_found_now().await,
            svc.read_only_folders(), self.peer.scope,
            svc.claude_key_exports_for(&self.peer),
        ))
    }

    /// A config write that failed, as something a form can show.
    ///
    /// Never the raw `io::Error`. A settings screen showing "Permission denied
    /// (os error 13)" has told the user nothing about which file or what to do,
    /// and the one thing they need to know is that nothing was changed.
    pub(super) fn config_write_failed(what: &str, error: std::io::Error) -> DomainError {
        tracing::warn!(%what, error = %error, "could not write config.toml");
        DomainError::OperationFailed
    }

    /// The runner's themes, in the shape every settings write answers with.
    ///
    /// Read back from the file rather than assembled from what was sent, so a
    /// client's list is what the file now says — including a color the writer
    /// normalized on the way in.
    pub(super) fn runner_themes() -> farcooler_protocol::v1::ThemeList {
        let items = farcooler_core::config::load_themes()
            .into_iter()
            .map(|t| farcooler_protocol::v1::Theme {
                name: t.name,
                dark: t.dark,
                background: t.background,
                foreground: t.foreground,
                cursor: t.cursor,
                ansi: t.ansi.to_vec(),
            })
            .collect();
        farcooler_protocol::v1::ThemeList { items }
    }

    /// A wire theme, validated.
    ///
    /// Exactly sixteen ANSI colors, and a name. The reader refuses a short list
    /// rather than padding it — "a color on screen that nobody chose and nobody
    /// can find in the file" — so the writer refuses one too, before it can
    /// produce a table the reader will then silently drop.
    pub(super) fn theme_from_wire(
        wire_theme: &farcooler_protocol::v1::Theme,
    ) -> Result<farcooler_core::theme::Theme> {
        let name = wire_theme.name.trim();
        farcooler_core::validate::display_name(name)?;
        if wire_theme.ansi.len() != 16 {
            return Err(DomainError::InvalidArgument { what: "ansi" });
        }
        let mut ansi = [0u32; 16];
        ansi.copy_from_slice(&wire_theme.ansi);
        Ok(farcooler_core::theme::Theme {
            name: name.to_string(),
            dark: wire_theme.dark,
            background: wire_theme.background,
            foreground: wire_theme.foreground,
            cursor: wire_theme.cursor,
            ansi,
        })
    }

}
