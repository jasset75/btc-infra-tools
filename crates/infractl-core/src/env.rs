use anyhow::{Result, bail};
use std::collections::HashMap;
use std::env;

/// Resolves logical variable names to concrete runtime values.
pub trait EnvResolver {
    fn resolve(&self, key: &str) -> Option<String>;
}

/// Production resolver backed by process environment variables.
pub struct ProcessEnvResolver;

impl EnvResolver for ProcessEnvResolver {
    fn resolve(&self, key: &str) -> Option<String> {
        env::var(key).ok()
    }
}

/// Test resolver backed by an in-memory map.
pub struct FixedEnvResolver {
    values: HashMap<String, String>,
}

impl FixedEnvResolver {
    pub fn new(values: HashMap<String, String>) -> Self {
        Self { values }
    }
}

impl EnvResolver for FixedEnvResolver {
    fn resolve(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }
}

/// Expands placeholder expressions in `input` using an injected value resolver.
///
/// Supported forms:
/// - `${VAR}`: requires resolver to return a value for `VAR`
/// - `${VAR:-default}`: falls back to `default` when resolver returns `None`
/// - `\${...}`: escaped placeholder, kept as a literal `${...}`
pub fn expand_placeholders(input: &str, resolver: &dyn EnvResolver) -> Result<String> {
    expand_placeholders_impl(input, &mut |key| resolver.resolve(key))
}

fn expand_placeholders_impl(
    input: &str,
    resolve: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if i + 2 < chars.len() && chars[i] == '\\' && chars[i + 1] == '$' && chars[i + 2] == '{' {
            out.push_str("${");
            i += 3;
            continue;
        }

        if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' {
            let start_expr = i + 2;
            let mut end_expr = None;
            let mut j = start_expr;
            while j < chars.len() {
                if chars[j] == '}' {
                    end_expr = Some(j);
                    break;
                }
                j += 1;
            }

            let Some(end_expr) = end_expr else {
                bail!("unterminated placeholder in `{input}`");
            };

            let expr: String = chars[start_expr..end_expr].iter().collect();
            if expr.is_empty() {
                bail!("empty placeholder in `{input}`");
            }

            let (key, default) = match expr.split_once(":-") {
                Some((k, d)) => (k, Some(d)),
                None => (expr.as_str(), None),
            };

            if key.is_empty() {
                bail!("empty placeholder key in `{input}`");
            }

            let value = resolve(key).or_else(|| default.map(ToOwned::to_owned)).ok_or_else(|| {
                anyhow::anyhow!("missing environment variable `{key}` for `{input}`")
            })?;

            out.push_str(&value);
            i = end_expr + 1;
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_expand_placeholders() {
        let mut values = HashMap::new();
        values.insert("FOO".to_string(), "bar".to_string());
        let resolver = FixedEnvResolver::new(values);
        let result = expand_placeholders("hello ${FOO} world", &resolver).unwrap();
        assert_eq!(result, "hello bar world");
    }

    #[test]
    fn test_expand_env_missing() {
        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = expand_placeholders("${MISSING_VAR_XYZ}", &resolver).unwrap_err();
        assert!(err.to_string().contains("missing environment variable `MISSING_VAR_XYZ`"));
    }

    #[test]
    fn test_expand_with_default_value() {
        let resolver = FixedEnvResolver::new(HashMap::new());
        let result = expand_placeholders("${MISSING_DEFAULT_VAR:-fallback}", &resolver).unwrap();
        assert_eq!(result, "fallback");
    }

    #[test]
    fn test_escape_placeholder() {
        let resolver = FixedEnvResolver::new(HashMap::new());
        let result = expand_placeholders(r"\${NOT_EXPANDED}", &resolver).unwrap();
        assert_eq!(result, "${NOT_EXPANDED}");
    }

    #[test]
    fn test_expand_with_fixed_env_resolver() {
        let mut values = HashMap::new();
        values.insert("FOO".to_string(), "bar".to_string());
        let resolver = FixedEnvResolver::new(values);
        let result = expand_placeholders("hello ${FOO}", &resolver).unwrap();
        assert_eq!(result, "hello bar");
    }
}
