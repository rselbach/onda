//! Portal connections carry the same identity as the desktop launcher.

use ashpd::zbus;

pub async fn connection() -> anyhow::Result<zbus::Connection> {
    let connection = zbus::Connection::session().await?;
    let registry = zbus::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.host.portal.Registry",
    )
    .await?;
    // Older portals identify host applications themselves and have no registry.
    if let Err(error) = registry
        .call::<_, _, ()>(
            "Register",
            &(
                super::APP_ID,
                std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new(),
            ),
        )
        .await
    {
        tracing::debug!(%error, "portal registry unavailable; using portal application detection");
    }
    Ok(connection)
}
