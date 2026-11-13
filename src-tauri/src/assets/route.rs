//! `eushully-asset://` request paths: `<session>/<entry index>/<kind>`.
//! The frontend builds URLs with `convertFileSrc`, which percent-encodes the
//! whole path (slashes included), so requests arrive encoded. Pure functions.

#[derive(Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// Full-size preview image, PNG.
    Image,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AssetRoute {
    pub session: u64,
    pub entry_index: usize,
    pub kind: AssetKind,
}

/// `%XX` → byte; invalid escapes are kept literally.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        let escape = (bytes[position] == b'%')
            .then(|| bytes.get(position + 1..position + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                decoded.push(byte);
                position += 3;
            }
            None => {
                decoded.push(bytes[position]);
                position += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Parse a request path (as received, possibly percent-encoded).
pub fn parse_route(raw_path: &str) -> Option<AssetRoute> {
    let path = percent_decode(raw_path);
    let mut parts = path.trim_matches('/').split('/');
    let session = parts.next()?.parse().ok()?;
    let entry_index = parts.next()?.parse().ok()?;
    let kind = match parts.next()? {
        "image" => AssetKind::Image,
        _ => return None,
    };
    parts.next().is_none().then_some(AssetRoute { session, entry_index, kind })
}

/// Launch request: `--browse <folder> [--show <file>]` opens Browse & Extract
/// on that game (and previews that file).
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseRequest {
    pub folder: String,
    pub show: Option<String>,
}

pub fn browse_request_from_args(args: impl IntoIterator<Item = String>) -> Option<BrowseRequest> {
    let (mut folder, mut show) = (None, None);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--browse" => folder = args.next(),
            "--show" => show = args.next(),
            _ => {}
        }
    }
    folder.map(|folder| BrowseRequest { folder, show })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_arguments() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            browse_request_from_args(args(&["app.exe", "--show", "A.AGF", "--browse", "C:/g"])),
            Some(BrowseRequest { folder: "C:/g".into(), show: Some("A.AGF".into()) })
        );
        assert_eq!(browse_request_from_args(args(&["app.exe", "--browse"])), None);
        assert_eq!(browse_request_from_args(args(&["app.exe", "--show", "x"])), None);
    }

    #[test]
    fn parses_encoded_and_plain_paths() {
        let expected = Some(AssetRoute { session: 3, entry_index: 1207, kind: AssetKind::Image });
        assert_eq!(parse_route("/3%2F1207%2Fimage"), expected);
        assert_eq!(parse_route("/3/1207/image"), expected);
    }

    #[test]
    fn rejects_malformed_paths() {
        assert_eq!(parse_route("/3/x/image"), None);
        assert_eq!(parse_route("/3/1/thumb"), None);
        assert_eq!(parse_route("/3/1/image/extra"), None);
        assert_eq!(percent_decode("a%zzb%41"), "a%zzbA");
    }
}
