use tracing::warn;

pub fn waiting(session_id: &str, cwd: Option<&str>, message: Option<&str>) {
    let body = format_body(session_id, cwd, message);
    if let Err(e) = notify_rust::Notification::new()
        .summary("Claude is waiting")
        .body(&body)
        .appname("cekanje")
        .show()
    {
        warn!(error = %e, "notify-rust failed");
    }
}

fn format_body(session_id: &str, cwd: Option<&str>, message: Option<&str>) -> String {
    match (cwd, message) {
        (Some(c), Some(m)) => format!("{c}\n{m}"),
        (Some(c), None) => c.to_string(),
        (None, Some(m)) => m.to_string(),
        (None, None) => session_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_body_with_cwd_and_message_joins_with_newline() {
        assert_eq!(
            format_body("sid", Some("/tmp/proj"), Some("hi there")),
            "/tmp/proj\nhi there"
        );
    }

    #[test]
    fn format_body_cwd_only_returns_cwd() {
        assert_eq!(format_body("sid", Some("/tmp/proj"), None), "/tmp/proj");
    }

    #[test]
    fn format_body_message_only_returns_message() {
        assert_eq!(format_body("sid", None, Some("hi")), "hi");
    }

    #[test]
    fn format_body_falls_back_to_session_id() {
        assert_eq!(format_body("sid-42", None, None), "sid-42");
    }
}
