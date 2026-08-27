//! Pure URI and local-resource reference resolution.

use url::Url;

/// Resolve `reference` against an opaque URL or local resource name.
#[must_use]
pub fn resolve_relative(base: &str, reference: &str) -> String {
    if Url::parse(reference).is_ok() {
        return reference.to_owned();
    }
    if let Ok(url) = Url::parse(base)
        && let Ok(resolved) = url.join(reference)
    {
        return resolved.to_string();
    }
    if reference.starts_with('/')
        || reference.starts_with('\\')
        || (reference.len() >= 2
            && reference.as_bytes()[1] == b':'
            && reference.as_bytes()[0].is_ascii_alphabetic())
    {
        return reference.to_owned();
    }
    let directory = base.rfind(['/', '\\']).map_or("", |index| &base[..index]);
    let directory = directory.trim_end_matches(['/', '\\']);
    if directory.is_empty() {
        reference.to_owned()
    } else {
        format!("{directory}/{reference}")
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_relative;

    #[test]
    fn resolves_urls_and_portable_local_references() {
        assert_eq!(resolve_relative("/a/b", "c/d"), "/a/c/d");
        assert_eq!(
            resolve_relative("C:\\foo\\bar\\tour.js", "tour.xml"),
            "C:\\foo\\bar/tour.xml"
        );
        assert_eq!(resolve_relative("http://a.b/x/", "c/d"), "http://a.b/x/c/d");
        assert_eq!(
            resolve_relative("/metadata/tour.xml", "/tiles/0_0.jpg"),
            "/tiles/0_0.jpg"
        );
    }
}
