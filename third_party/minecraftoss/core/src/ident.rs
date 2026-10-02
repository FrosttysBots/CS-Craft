//! Namespaced identifiers (`namespace:path`).

use std::fmt;
use std::sync::Arc;

/// A namespaced identifier. Cheap to clone.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identifier(Arc<str>);

impl Identifier {
    /// Parses `namespace:path`, defaulting the namespace to `minecraft`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (namespace, path) = text.split_once(':').unwrap_or(("minecraft", text));
        let valid_namespace = |c: char| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.');
        let valid_path = |c: char| valid_namespace(c) || c == '/';
        if namespace.is_empty()
            || path.is_empty()
            || !namespace.chars().all(valid_namespace)
            || !path.chars().all(valid_path)
        {
            return Err(format!("invalid identifier `{text}`"));
        }
        Ok(Self(Arc::from(format!("{namespace}:{path}"))))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn namespace(&self) -> &str {
        self.0.split_once(':').expect("validated identifier").0
    }

    pub fn path(&self) -> &str {
        self.0.split_once(':').expect("validated identifier").1
    }
}

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_namespace_and_rejects_invalid() {
        let id = Identifier::parse("stone").unwrap();
        assert_eq!(id.as_str(), "minecraft:stone");
        assert_eq!((id.namespace(), id.path()), ("minecraft", "stone"));
        assert!(Identifier::parse("Minecraft:Stone").is_err());
        assert!(Identifier::parse("minecraft:").is_err());
    }
}
