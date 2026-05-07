use tracing::warn;

pub fn waiting(session_id: &str, cwd: Option<&str>, message: Option<&str>) {
    let title = "Claude is waiting";
    let body = match (cwd, message) {
        (Some(c), Some(m)) => format!("{c}\n{m}"),
        (Some(c), None) => c.to_string(),
        (None, Some(m)) => m.to_string(),
        (None, None) => session_id.to_string(),
    };
    if let Err(e) = notify_rust::Notification::new()
        .summary(title)
        .body(&body)
        .appname("cekanje")
        .show()
    {
        warn!(error = %e, "notify-rust failed");
    }
}
