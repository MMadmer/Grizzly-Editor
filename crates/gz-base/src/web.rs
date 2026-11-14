//! Web addresses, as far as the editor judges them itself.

/// Whether an address is this computer's: `http://127.0.0.1` or `http://localhost`, with a port
/// or without, and nothing else. A QA run speaks only to such an address, so the test reads the
/// host the way a web client does and refuses what could hide another one: an authority with
/// user information (`http://127.0.0.1:1@example.com` is example.com), or with any character a
/// host and a port do not have.
#[must_use]
pub fn is_loopback(url: &str) -> bool {
    let url = url.trim();
    let Some(rest) = url.get(..7).filter(|s| s.eq_ignore_ascii_case("http://")).map(|_| &url[7..]) else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || !authority.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b':')
    {
        return false;
    }
    let host = match authority.split_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return false,
        None => authority,
    };
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost")
}

#[cfg(test)]
mod tests {
    use super::is_loopback;

    #[test]
    fn only_this_computer_is_loopback() {
        for yes in [
            "http://127.0.0.1:5000/x",
            "http://localhost:80",
            "HTTP://LocalHost:8080/a?b#c",
            "http://127.0.0.1",
            "http://127.0.0.1:9/",
        ] {
            assert!(is_loopback(yes), "{yes}");
        }
        for no in [
            "https://api.github.com/repos",
            "https://127.0.0.1:5000/",
            "http://127.0.0.1.example.com:5000/x",
            "http://localhostile:5/",
            // user information: the host is what follows the @
            "http://127.0.0.1:1@api.github.com/x",
            "http://localhost@example.com/",
            "http://127.0.0.1\\@example.com/",
            "http://127.0.0.1:port/",
            "http://127.0.0.1:/",
            "http://[::1]:5000/",
            "127.0.0.1:5000",
            "http://",
            "",
        ] {
            assert!(!is_loopback(no), "{no}");
        }
    }
}
