use std::{collections::HashMap, env, path::PathBuf};

use url::{Host, Url};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeEnvironment {
    Development,
    Production,
}

impl RuntimeEnvironment {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "development" => Ok(Self::Development),
            "production" => Ok(Self::Production),
            _ => Err("CIRCUS_ENVIRONMENT must be development or production".to_owned()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeConfig {
    environment: RuntimeEnvironment,
    public_origin: String,
}

impl RuntimeConfig {
    #[cfg(test)]
    pub(crate) fn development_for_test() -> Self {
        Self {
            environment: RuntimeEnvironment::Development,
            public_origin: "http://localhost:4000".to_owned(),
        }
    }

    pub(crate) fn environment(&self) -> RuntimeEnvironment {
        self.environment
    }

    pub(crate) fn public_origin(&self) -> &str {
        &self.public_origin
    }

    pub(crate) fn uses_secure_cookies(&self) -> bool {
        self.environment != RuntimeEnvironment::Development
    }
}

/// Validated process configuration.
pub(crate) struct StartupConfig {
    pub(crate) bind_address: String,
    pub(crate) runtime: RuntimeConfig,
    pub(crate) static_dir: PathBuf,
    pub(crate) data_dir: PathBuf,
}

impl StartupConfig {
    pub(crate) fn from_environment() -> Result<Self, String> {
        Self::from_values(env::vars().collect())
    }

    fn from_values(values: HashMap<String, String>) -> Result<Self, String> {
        let environment = RuntimeEnvironment::parse(&required(&values, "CIRCUS_ENVIRONMENT")?)?;
        let public_origin = match environment {
            RuntimeEnvironment::Development => values
                .get("CIRCUS_PUBLIC_ORIGIN")
                .cloned()
                .unwrap_or_else(|| "http://localhost:4000".to_owned()),
            RuntimeEnvironment::Production => required(&values, "CIRCUS_PUBLIC_ORIGIN")?,
        };
        let runtime = RuntimeConfig {
            environment,
            public_origin: parse_public_origin(&public_origin)?,
        };
        let bind_address = values
            .get("CIRCUS_BIND_ADDRESS")
            .map(String::as_str)
            .unwrap_or("localhost:4000")
            .to_owned();
        let static_dir = match values.get("CIRCUS_STATIC_DIR") {
            Some(path) if PathBuf::from(path).is_absolute() => PathBuf::from(path),
            Some(_) => return Err("CIRCUS_STATIC_DIR must be absolute".to_owned()),
            None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static"),
        };
        Ok(Self {
            bind_address,
            runtime,
            static_dir,
            data_dir: parse_data_dir(&values)?,
        })
    }
}

fn required(values: &HashMap<String, String>, name: &str) -> Result<String, String> {
    match values.get(name) {
        Some(value) if !value.is_empty() => Ok(value.clone()),
        Some(_) => Err(format!("{name} must not be empty")),
        None => Err(format!("{name} must be set")),
    }
}

fn parse_data_dir(values: &HashMap<String, String>) -> Result<PathBuf, String> {
    if let Some(path) = values.get("CIRCUS_DATA_DIR") {
        if path.is_empty() {
            return Err("CIRCUS_DATA_DIR must not be empty".to_owned());
        }
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("CIRCUS_DATA_DIR must be absolute".to_owned());
        }
        return Ok(path);
    }
    if let Some(xdg) = values
        .get("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(xdg).join("circus"));
    }
    if let Some(home) = values.get("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home).join(".local/share/circus"));
    }
    Err("CIRCUS_DATA_DIR must be set".to_owned())
}

fn parse_public_origin(value: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "CIRCUS_PUBLIC_ORIGIN is invalid".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("CIRCUS_PUBLIC_ORIGIN must be a canonical HTTP(S) origin".to_owned());
    }
    let host = match url.host().expect("host was checked above") {
        Host::Domain(domain) => domain.to_string(),
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => format!("[{address}]"),
    };
    let canonical = match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    };
    if value != canonical {
        return Err("CIRCUS_PUBLIC_ORIGIN must be canonical".to_owned());
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests;
