//! The one check for URLs that a module's content may open: the plugin `open` import and links
//! clicked in markdown both use [`checked_url`], and open the URL it returns, never the text
//! they were given.

use url::Url;

/// The `mailto` fields that are kept (Ruling AC: no `attach`).
const MAILTO_FIELDS: [&str; 5] = ["to", "cc", "bcc", "subject", "body"];

/// `text` as the URL to open, in its parsed and filtered form, if the rules allow it:
/// - no whitespace or control characters, and it parses;
/// - the scheme is `http`, `https` or `mailto`;
/// - no user name or password;
/// - a `mailto` keeps only its `to`, `cc`, `bcc`, `subject` and `body` fields, as written.
pub fn checked_url(text: &str) -> Result<String, String> {
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a URL may not contain spaces or control characters".into());
    }
    let mut url = Url::parse(text).map_err(|e| format!("not a valid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https" | "mailto") {
        return Err("only http, https and mailto URLs may be opened".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with a user name or password may not be opened".into());
    }
    if url.scheme() == "mailto" {
        keep_mail_fields(&mut url);
    }
    Ok(url.into())
}

/// Drops every `mailto` field but [`MAILTO_FIELDS`], keeping the kept ones as written.
fn keep_mail_fields(url: &mut Url) {
    let Some(query) = url.query() else {
        return;
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|field| {
            let key = field.split('=').next().unwrap_or_default();
            let key = percent_decode(key).to_ascii_lowercase();
            MAILTO_FIELDS.contains(&key.as_str())
        })
        .collect();
    let kept = kept.join("&");
    url.set_query((!kept.is_empty()).then_some(kept.as_str()));
}

/// `%XX` escapes decoded (lossy), for comparing a field name.
fn percent_decode(text: &str) -> String {
    url::form_urlencoded::parse(format!("{text}=").as_bytes())
        .next()
        .map(|(key, _)| key.into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_need_an_allowed_scheme_and_no_credentials() {
        for ok in [
            "https://example.com/",
            "http://example.com/a?b=c#d",
            "mailto:a@example.com",
            "MAILTO:a@example.com",
        ] {
            assert!(checked_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vbscript:x",
            "data:text/plain,x",
            "ftp://example.com/",
            "x-scheme-handler:foo",
            "steam://run/1",
            "https://u:p@example.com/",
            "https://u@example.com/",
            " https://example.com/",
            "https://example.com/a b",
            "https://example.com/\u{0}",
            "https://example.com/\u{85}",
            "/etc/passwd",
            "",
        ] {
            assert!(checked_url(bad).is_err(), "{bad:?}");
        }
    }

    /// Ruling AC.
    #[test]
    fn mailto_keeps_only_the_message_fields() {
        let open = |s: &str| checked_url(s).unwrap();
        assert_eq!(
            open("mailto:a@example.com?subject=Hi%20there&attach=/etc/passwd&body=x"),
            "mailto:a@example.com?subject=Hi%20there&body=x"
        );
        assert_eq!(
            open(
                "mailto:a@example.com?Attachment=x&%61ttach=y&CC=b@example.com&bcc=c@x.y&to=d@x.y"
            ),
            "mailto:a@example.com?CC=b@example.com&bcc=c@x.y&to=d@x.y"
        );
        assert_eq!(
            open("mailto:a@example.com?attach=x"),
            "mailto:a@example.com"
        );
        assert_eq!(open("mailto:a@example.com"), "mailto:a@example.com");
        // Other schemes keep their query.
        assert_eq!(
            open("https://example.com/?attach=x"),
            "https://example.com/?attach=x"
        );
    }
}
