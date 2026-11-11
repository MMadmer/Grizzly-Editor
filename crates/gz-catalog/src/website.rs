//! The module's own page the Mod Browser shows (`EditorCatalog::WebsiteIsModulePage`): an AP-PRO
//! file page or forum topic, or a ModDB mod / addon page.

fn slug_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

/// A run of `allowed` bytes up to `/`, `?`, `#` or the end; `None` when it is empty.
fn segment(p: &[u8], allowed: fn(u8) -> bool) -> Option<(&[u8], &[u8])> {
    let n = p.iter().take_while(|c| !matches!(**c, b'/' | b'?' | b'#') && allowed(**c)).count();
    (n > 0).then(|| p.split_at(n))
}

fn starts<'a>(p: &'a [u8], prefix: &str) -> Option<&'a [u8]> {
    let head = p.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix.as_bytes()).then(|| p.get(prefix.len()..).unwrap_or(&[]))
}

// what may follow an AP-PRO page: an optional slash, then nothing or a query / fragment
fn page_end(p: &[u8]) -> bool {
    let p = p.strip_prefix(b"/").unwrap_or(p);
    matches!(p.first(), None | Some(b'?' | b'#'))
}

/// Whether `url` is the module's own page on AP-PRO or ModDB.
#[must_use]
pub fn website_is_module_page(url: &str) -> bool {
    if url.is_empty() || !gz_mod::validate_website(url) {
        return false;
    }
    let Some(p) = starts(url.as_bytes(), "https://") else { return false };
    let p = starts(p, "www.").unwrap_or(p);
    if let Some(p) = starts(p, "ap-pro.ru/") {
        // /stuff/<section>/<slug>-r<number>/ or /forums/topic/<number>-<slug>/
        if let Some(p) = starts(p, "stuff/") {
            let Some((_, p)) = segment(p, slug_char) else { return false };
            let Some(p) = p.strip_prefix(b"/") else { return false };
            let Some((last, p)) = segment(p, slug_char) else { return false };
            let Some(r) = last.windows(2).rposition(|w| w == b"-r") else { return false };
            let number = last.get(r + 2..).unwrap_or(&[]);
            return !number.is_empty() && number.iter().all(u8::is_ascii_digit) && page_end(p);
        }
        if let Some(p) = starts(p, "forums/topic/") {
            let digits = p.iter().take_while(|c| c.is_ascii_digit()).count();
            let Some(p) = p.get(digits..).filter(|_| digits > 0).and_then(|p| p.strip_prefix(b"-")) else {
                return false;
            };
            return segment(p, slug_char).is_some_and(|(_, rest)| page_end(rest));
        }
        return false;
    }
    // a ModDB page may go on into its own sub-pages (/downloads, /images), not into a query
    if let Some(p) = starts(p, "moddb.com/") {
        let tail_ok = |rest: &[u8]| matches!(rest.first(), None | Some(b'/'));
        if let Some(p) = starts(p, "mods/").or_else(|| starts(p, "addons/")) {
            return segment(p, slug_char).is_some_and(|(_, rest)| tail_ok(rest));
        }
        if let Some(p) = starts(p, "games/") {
            let Some((_, p)) = segment(p, slug_char) else { return false };
            let Some(p) = starts(p, "/addons/") else { return false };
            return segment(p, slug_char).is_some_and(|(_, rest)| tail_ok(rest));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_pages() {
        assert!(website_is_module_page("https://ap-pro.ru/stuff/mods/overnight_delivery-r123/"));
        assert!(website_is_module_page("https://www.ap-pro.ru/stuff/mods/od-r1"));
        assert!(website_is_module_page("https://ap-pro.ru/forums/topic/4242-overnight-delivery/"));
        assert!(website_is_module_page("https://www.moddb.com/mods/overnight-delivery/downloads"));
        assert!(website_is_module_page("https://www.moddb.com/games/stalker/addons/od"));
        assert!(!website_is_module_page("https://ap-pro.ru/stuff/mods/"));
        assert!(!website_is_module_page("https://ap-pro.ru/stuff/mods/od/"));
        assert!(!website_is_module_page("https://ap-pro.ru/stuff/mods/od-rx/"));
        assert!(!website_is_module_page("https://ap-pro.ru/forums/topic/-od/"));
        assert!(!website_is_module_page("https://www.moddb.com/mods/od?x=1"));
        assert!(!website_is_module_page("http://ap-pro.ru/stuff/mods/od-r1/"));
        assert!(!website_is_module_page("https://example.com/stuff/mods/od-r1/"));
        assert!(!website_is_module_page(""));
    }
}
