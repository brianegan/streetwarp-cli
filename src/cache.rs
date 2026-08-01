//! A content-addressed cache for Google API responses.
//!
//! Every response this program fetches is a pure function of its request URL, so
//! the URL is the cache key. Re-rendering a route then costs nothing at Google
//! instead of paying for the same frames twice.

use std::fs;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

/// FNV-1a offset basis and prime for the 64-bit variant.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Hash bytes with FNV-1a, 64-bit.
///
/// The cache outlives the toolchain that wrote it, so the key has to mean the
/// same thing next year. `DefaultHasher` is explicitly documented as unstable
/// between Rust releases, which would silently orphan every cached image on a
/// toolchain bump. FNV-1a is a fixed published algorithm, so it cannot drift.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Strip the `key` query parameter from a URL, leaving everything else in place.
///
/// Two runs with different API keys ask Google for identical bytes, so the key
/// must not reach the cache key. It also means the secret never lands in a
/// filename.
pub fn without_api_key(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let kept = query
        .split('&')
        .filter(|param| param.split_once('=').map(|(name, _)| name) != Some("key"))
        .collect::<Vec<_>>()
        .join("&");
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{kept}")
    }
}

/// Replace the value of any `key=` parameter appearing anywhere in `text`.
///
/// Nothing here formats a request URL into a message deliberately, but reqwest
/// puts the failing URL into its own error `Display`, so an error string carries
/// the API key without anyone asking it to. That string reaches a panic and the
/// terminal, so it gets scrubbed at the point the error is built rather than
/// trusted not to travel.
pub fn redact_api_key(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("key=") {
        let (before, from_key) = rest.split_at(at);
        // Only a parameter when it starts one, so `monkey=1` is left alone.
        let starts_parameter = before.is_empty()
            || matches!(before.as_bytes()[before.len() - 1], b'?' | b'&');
        out.push_str(before);
        out.push_str("key=");
        let value = &from_key["key=".len()..];
        if !starts_parameter {
            rest = value;
            continue;
        }
        out.push_str("REDACTED");
        let end = value
            .find(|c: char| c == '&' || c == ')' || c.is_whitespace())
            .unwrap_or(value.len());
        rest = &value[end..];
    }
    out.push_str(rest);
    out
}

/// What sort of response is being cached. Each kind gets its own subdirectory
/// and file extension, so the cache stays browsable by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    StreetView,
    Map,
    Metadata,
}

impl Kind {
    fn dir(self) -> &'static str {
        match self {
            Kind::StreetView => "sv",
            Kind::Map => "map",
            Kind::Metadata => "meta",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Kind::StreetView => "jpg",
            Kind::Map => "png",
            Kind::Metadata => "json",
        }
    }
}

/// A cache of Google API responses on disk.
///
/// A disabled cache reads nothing and writes nothing, which is what `--no-cache`
/// produces.
pub struct Cache {
    root: Option<PathBuf>,
}

impl Cache {
    /// A cache under the platform cache directory, or disabled if that
    /// directory cannot be located.
    pub fn in_platform_cache_dir() -> Cache {
        Cache {
            root: dirs::cache_dir().map(|d| d.join("streetwarp")),
        }
    }

    /// A cache under an explicit root, so a test gets a scratch directory of its
    /// own instead of sharing the real one.
    #[cfg(test)]
    pub fn rooted_at<P: AsRef<Path>>(root: P) -> Cache {
        Cache {
            root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// A cache that never stores or returns anything.
    pub fn disabled() -> Cache {
        Cache { root: None }
    }

    /// Where a response for `url` would live, or `None` when disabled.
    pub fn path_for(&self, kind: Kind, url: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let digest = format!("{:016x}", fnv1a_64(without_api_key(url).as_bytes()));
        // Shard on the first byte so no single directory collects every frame
        // of every route ever rendered.
        Some(
            root.join(kind.dir())
                .join(&digest[..2])
                .join(format!("{digest}.{}", kind.extension())),
        )
    }

    /// Read a cached response, or `None` on a miss.
    pub fn get(&self, kind: Kind, url: &str) -> Option<Vec<u8>> {
        fs::read(self.path_for(kind, url)?).ok()
    }

    /// Store a response. Failures are reported and swallowed: an unwritable
    /// cache should slow a re-render down, not fail the one running now.
    pub fn put(&self, kind: Kind, url: &str, bytes: &[u8]) {
        let Some(path) = self.path_for(kind, url) else {
            return;
        };
        let write = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&path, bytes));
        if let Err(err) = write {
            eprintln!("Could not cache {}: {err}", path.display());
        }
    }
}
