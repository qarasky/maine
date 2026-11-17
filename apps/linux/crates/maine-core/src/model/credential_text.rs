use super::Host;

/// The plain-text credential block copied for a host: one labelled field per
/// line, only the fields that exist.
pub fn credential_text(host: &Host, password: Option<&str>) -> String {
    let mut lines = vec![format!("IP: {}", host.hostname)];
    if host.port != Host::DEFAULT_PORT {
        lines.push(format!("port: {}", host.port));
    }
    if !host.username.is_empty() {
        lines.push(format!("username: {}", host.username));
    }
    if let Some(password) = password.filter(|p| !p.is_empty()) {
        lines.push(format!("password: {password}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_becomes_one_labelled_line() {
        let mut h = Host::new("Web", "10.0.0.4");
        h.port = 2222;
        h.username = "root".into();
        assert_eq!(
            credential_text(&h, Some("abcdef123")),
            "IP: 10.0.0.4\nport: 2222\nusername: root\npassword: abcdef123"
        );
    }

    #[test]
    fn default_port_and_missing_fields_are_omitted() {
        let h = Host::new("Web", "example.com");
        assert_eq!(credential_text(&h, None), "IP: example.com");
        assert_eq!(credential_text(&h, Some("")), "IP: example.com");
    }

    #[test]
    fn username_survives_without_password() {
        let mut h = Host::new("Web", "example.com");
        h.username = "deploy".into();
        assert_eq!(
            credential_text(&h, None),
            "IP: example.com\nusername: deploy"
        );
    }
}
